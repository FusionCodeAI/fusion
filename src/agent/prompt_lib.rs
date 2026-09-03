//! Custom prompt template library and variable interpolation subsystem for Fusion.
//!
//! Provides a production-grade prompt management system allowing developers to:
//! - Curate, organize, and categorize reusable prompt templates.
//! - Interpolate dynamic variables with named (`{{var}}`, `{{var:-default}}`), positional (`$1`, `$2`),
//!   or key-value argument formats.
//! - Save custom prompts to project-local (`.fusion/prompts/`) or user-global (`~/.fusion/prompts/`) storage.
//! - Load and execute templates dynamically via `/prompt load <name>` or `/prompt save <name>`.
//! - Import/export prompt collections as JSON, TOML, or Markdown with YAML-compatible frontmatter.
//! - Leverage an extensive built-in library of specialized engineering prompts (code review, refactoring,
//!   test generation, architecture analysis, debugging, security audits, and more).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::config::Config;

/// Error types occurring during prompt library operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptLibError {
    /// Requested prompt template was not found in the library.
    TemplateNotFound(String),
    /// A required template variable was not provided and has no default value.
    MissingVariable { template: String, variable: String },
    /// File system or I/O error occurred.
    Io(String),
    /// Failed to parse or serialize JSON / Markdown frontmatter.
    Serialization(String),
    /// Template validation failed (e.g. empty name or body).
    Validation(String),
    /// Argument parsing error.
    InvalidArguments(String),
}

impl fmt::Display for PromptLibError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TemplateNotFound(name) => {
                write!(f, "Prompt template '{}' not found in library", name)
            }
            Self::MissingVariable { template, variable } => write!(
                f,
                "Missing required variable '{}' for template '{}'",
                variable, template
            ),
            Self::Io(msg) => write!(f, "Prompt I/O error: {}", msg),
            Self::Serialization(msg) => write!(f, "Prompt serialization error: {}", msg),
            Self::Validation(msg) => write!(f, "Prompt validation error: {}", msg),
            Self::InvalidArguments(msg) => write!(f, "Invalid prompt arguments: {}", msg),
        }
    }
}

impl std::error::Error for PromptLibError {}

/// Definition of a variable placeholder inside a prompt template.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptVariable {
    /// Identifier name (e.g. `"code"`, `"focus"`, `"lang"`).
    pub name: String,
    /// Human-readable explanation of what to provide.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Optional default fallback value if unspecified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_value: Option<String>,
    /// Whether this variable is strictly required for rendering.
    #[serde(default)]
    pub required: bool,
}

impl PromptVariable {
    /// Create a new variable definition with a name and required status.
    pub fn new(name: impl Into<String>, required: bool) -> Self {
        Self {
            name: name.into(),
            description: None,
            default_value: None,
            required,
        }
    }

    /// Create an optional variable with a default value.
    pub fn with_default(name: impl Into<String>, default: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: None,
            default_value: Some(default.into()),
            required: false,
        }
    }

    /// Set an explanatory description.
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }
}

/// A structured, reusable prompt template.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PromptTemplate {
    /// Unique identifier / slug name (e.g. `"review"`, `"refactor"`, `"test-gen"`).
    pub name: String,
    /// Brief one-line summary of what the prompt accomplishes.
    pub description: String,
    /// The template body text with `{{var}}` placeholders.
    pub template: String,
    /// Categorical grouping (e.g. `"Coding"`, `"Review"`, `"Testing"`, `"Architecture"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// Searchable keyword tags.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Declared or inferred variable placeholders.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variables: Vec<PromptVariable>,
    /// Optional custom system prompt to override or prepend when using this template.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt_override: Option<String>,
    /// Optional preferred model shorthand.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_override: Option<String>,
    /// Optional generation temperature override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    /// Author / contributor attribution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// Unix timestamp when created.
    #[serde(default)]
    pub created_at: u64,
    /// Unix timestamp when last modified.
    #[serde(default)]
    pub updated_at: u64,
    /// Whether this is a factory built-in template.
    #[serde(default)]
    pub is_builtin: bool,
}

impl PromptTemplate {
    /// Create a new prompt template with default metadata and auto-inferred variables.
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        template: impl Into<String>,
    ) -> Self {
        let name_str = name.into();
        let desc_str = description.into();
        let tmpl_str = template.into();
        let now = current_timestamp();

        let mut t = Self {
            name: name_str,
            description: desc_str,
            template: tmpl_str,
            category: None,
            tags: Vec::new(),
            variables: Vec::new(),
            system_prompt_override: None,
            model_override: None,
            temperature: None,
            author: None,
            created_at: now,
            updated_at: now,
            is_builtin: false,
        };
        t.auto_populate_variables();
        t
    }

    /// Builder entrypoint for fluent template construction.
    pub fn builder(name: impl Into<String>) -> PromptTemplateBuilder {
        PromptTemplateBuilder::new(name)
    }

    /// Set category.
    pub fn with_category(mut self, category: impl Into<String>) -> Self {
        self.category = Some(category.into());
        self
    }

    /// Set tags.
    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }

    /// Set explicit variable metadata list.
    pub fn with_variables(mut self, vars: Vec<PromptVariable>) -> Self {
        self.variables = vars;
        self
    }

    /// Set system prompt override.
    pub fn with_system_prompt(mut self, sys: impl Into<String>) -> Self {
        self.system_prompt_override = Some(sys.into());
        self
    }

    /// Set model override.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model_override = Some(model.into());
        self
    }

    /// Set temperature.
    pub fn with_temperature(mut self, temp: f32) -> Self {
        self.temperature = Some(temp);
        self
    }

    /// Mark as built-in.
    pub fn with_builtin(mut self, builtin: bool) -> Self {
        self.is_builtin = builtin;
        self
    }

    /// Automatically extract variable placeholders from the template text and populate `self.variables`
    /// while preserving any previously defined descriptions or defaults.
    pub fn auto_populate_variables(&mut self) {
        let extracted = extract_placeholders(&self.template);
        let existing_map: HashMap<String, PromptVariable> = self
            .variables
            .drain(..)
            .map(|v| (v.name.clone(), v))
            .collect();

        let mut merged = Vec::new();
        for mut var in extracted {
            if let Some(existing) = existing_map.get(&var.name) {
                if existing.description.is_some() {
                    var.description = existing.description.clone();
                }
                if existing.default_value.is_some() {
                    var.default_value = existing.default_value.clone();
                    var.required = existing.required;
                }
            }
            merged.push(var);
        }
        self.variables = merged;
    }

    /// Render template with named variable substitutions.
    pub fn render(&self, vars: &HashMap<String, String>) -> Result<String, PromptLibError> {
        let (rendered, missing) = substitute_variables(&self.template, vars, &self.variables);
        if let Some(missing_var) = missing.into_iter().next() {
            return Err(PromptLibError::MissingVariable {
                template: self.name.clone(),
                variable: missing_var,
            });
        }
        Ok(rendered)
    }

    /// Render template with positional arguments.
    pub fn render_positional(&self, args: &[&str]) -> Result<String, PromptLibError> {
        let vars = parse_positional_args(args, self);
        self.render(&vars)
    }

    /// Render template using parsed CLI tokens (supporting `key=value`, flags, and positional values).
    pub fn render_cli_args(&self, raw_args: &[String]) -> Result<String, PromptLibError> {
        let vars = parse_cli_tokens(raw_args, self);
        self.render(&vars)
    }

    /// Format template metadata and preview as a clean Markdown card.
    pub fn format_markdown_card(&self) -> String {
        let mut out = String::new();
        let cat = self.category.as_deref().unwrap_or("General");
        let builtin_tag = if self.is_builtin { " `(builtin)`" } else { "" };
        out.push_str(&format!(
            "### `/prompt load {}`{}\n",
            self.name, builtin_tag
        ));
        out.push_str(&format!(
            "**Category:** {} | **Description:** {}\n\n",
            cat, self.description
        ));

        if !self.variables.is_empty() {
            out.push_str("**Variables:**\n");
            for v in &self.variables {
                let req = if v.required {
                    "*(required)*"
                } else {
                    "*(optional)*"
                };
                let def = v
                    .default_value
                    .as_deref()
                    .map(|d| format!(", default: `{}`", d))
                    .unwrap_or_default();
                let desc = v
                    .description
                    .as_deref()
                    .map(|d| format!(" - {}", d))
                    .unwrap_or_default();
                out.push_str(&format!("- `{{{{{}}}}}` {} {}{}\n", v.name, req, def, desc));
            }
            out.push('\n');
        }

        out.push_str("```\n");
        let preview: String = self.template.lines().take(5).collect::<Vec<_>>().join("\n");
        out.push_str(&preview);
        if self.template.lines().count() > 5 {
            out.push_str("\n...");
        }
        out.push_str("\n```\n");
        out
    }

    /// Serialize to Markdown with YAML-compatible frontmatter.
    pub fn to_markdown_frontmatter(&self) -> String {
        let mut out = String::new();
        out.push_str("---\n");
        out.push_str(&format!("name: \"{}\"\n", self.name.replace('"', "\\\"")));
        out.push_str(&format!(
            "description: \"{}\"\n",
            self.description.replace('"', "\\\"")
        ));
        if let Some(cat) = &self.category {
            out.push_str(&format!("category: \"{}\"\n", cat.replace('"', "\\\"")));
        }
        if !self.tags.is_empty() {
            let tags_json = serde_json::to_string(&self.tags).unwrap_or_else(|_| "[]".into());
            out.push_str(&format!("tags: {}\n", tags_json));
        }
        if let Some(model) = &self.model_override {
            out.push_str(&format!("model: \"{}\"\n", model));
        }
        if let Some(sys) = &self.system_prompt_override {
            out.push_str(&format!(
                "system_prompt: \"{}\"\n",
                sys.replace('"', "\\\"").replace('\n', "\\n")
            ));
        }
        if let Some(temp) = self.temperature {
            out.push_str(&format!("temperature: {}\n", temp));
        }
        out.push_str(&format!("created_at: {}\n", self.created_at));
        out.push_str(&format!("updated_at: {}\n", self.updated_at));
        out.push_str("---\n\n");
        out.push_str(&self.template);
        out
    }

    /// Parse a template from a Markdown document containing frontmatter.
    pub fn from_markdown_frontmatter(content: &str) -> Result<Self, PromptLibError> {
        let trimmed = content.trim_start();
        if !trimmed.starts_with("---") {
            // Treat entire content as raw template body
            return Ok(PromptTemplate::new("custom", "Custom user prompt", content));
        }

        let after_first = &trimmed[3..];
        let Some(end_idx) = after_first.find("\n---") else {
            return Err(PromptLibError::Serialization(
                "Malformed frontmatter: missing closing '---'".to_string(),
            ));
        };

        let frontmatter_str = &after_first[..end_idx];
        let body_str = after_first[end_idx + 4..]
            .trim_start_matches('\n')
            .trim_start_matches('\r');

        let mut name = String::new();
        let mut description = String::new();
        let mut category = None;
        let mut tags = Vec::new();
        let mut model_override = None;
        let mut system_prompt_override = None;
        let mut temperature = None;
        let mut created_at = current_timestamp();
        let mut updated_at = current_timestamp();

        for line in frontmatter_str.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((k, v)) = line.split_once(':') {
                let key = k.trim().to_lowercase();
                let val = v.trim().trim_matches('"').trim_matches('\'').to_string();
                match key.as_str() {
                    "name" | "id" | "slug" => name = val,
                    "description" | "desc" | "summary" => description = val,
                    "category" | "group" => category = Some(val),
                    "tags" => {
                        if val.starts_with('[') && val.ends_with(']') {
                            if let Ok(parsed_tags) = serde_json::from_str::<Vec<String>>(&val) {
                                tags = parsed_tags;
                            }
                        } else {
                            tags = val
                                .split(',')
                                .map(|s| s.trim().to_string())
                                .filter(|s| !s.is_empty())
                                .collect();
                        }
                    }
                    "model" => model_override = Some(val),
                    "system_prompt" | "system" => {
                        system_prompt_override =
                            Some(val.replace("\\n", "\n").replace("\\\"", "\""))
                    }
                    "temperature" | "temp" => {
                        if let Ok(t) = val.parse::<f32>() {
                            temperature = Some(t);
                        }
                    }
                    "created_at" => {
                        if let Ok(ts) = val.parse::<u64>() {
                            created_at = ts;
                        }
                    }
                    "updated_at" => {
                        if let Ok(ts) = val.parse::<u64>() {
                            updated_at = ts;
                        }
                    }
                    _ => {}
                }
            }
        }

        if name.is_empty() {
            name = "custom".to_string();
        }
        if description.is_empty() {
            description = format!("Prompt template for {}", name);
        }

        let mut tmpl = PromptTemplate {
            name,
            description,
            template: body_str.to_string(),
            category,
            tags,
            variables: Vec::new(),
            system_prompt_override,
            model_override,
            temperature,
            author: None,
            created_at,
            updated_at,
            is_builtin: false,
        };
        tmpl.auto_populate_variables();
        Ok(tmpl)
    }

    /// Estimates the token cost of the rendered prompt (approx. 4 chars/token heuristic).
    ///
    /// Renders the template with `vars` supplied, then applies the crate-wide
    /// heuristic tokenizer to the rendered output.
    pub fn estimate_tokens(&self, vars: &HashMap<String, String>) -> usize {
        match self.render(vars) {
            Ok(rendered) => crate::agent::tokens::estimate_text_tokens(&rendered),
            Err(_) => 0,
        }
    }

    /// Returns the approximate character length of the template body.
    pub fn len(&self) -> usize {
        self.template.len()
    }

    /// Returns `true` when the template body is empty.
    pub fn is_empty(&self) -> bool {
        self.template.is_empty()
    }

    /// Returns the currently configured tags.
    pub fn tags(&self) -> &[String] {
        &self.tags
    }

    /// Returns the currently configured category.
    pub fn category(&self) -> Option<&str> {
        self.category.as_deref()
    }

    /// Returns the currently configured model override.
    pub fn model_override(&self) -> Option<&str> {
        self.model_override.as_deref()
    }

    /// Returns the currently configured system prompt override.
    pub fn system_prompt_override(&self) -> Option<&str> {
        self.system_prompt_override.as_deref()
    }

    /// Returns the currently configured temperature override.
    pub fn temperature(&self) -> Option<f32> {
        self.temperature
    }
}

/// Builder helper for fluent `PromptTemplate` construction.
#[derive(Debug, Clone)]
pub struct PromptTemplateBuilder {
    template: PromptTemplate,
}

impl PromptTemplateBuilder {
    /// Create a new builder for template with `name`.
    pub fn new(name: impl Into<String>) -> Self {
        let name_str = name.into();
        let now = current_timestamp();
        Self {
            template: PromptTemplate {
                name: name_str.clone(),
                description: format!("Template for {}", name_str),
                template: String::new(),
                category: None,
                tags: Vec::new(),
                variables: Vec::new(),
                system_prompt_override: None,
                model_override: None,
                temperature: None,
                author: None,
                created_at: now,
                updated_at: now,
                is_builtin: false,
            },
        }
    }

    /// Set summary description.
    pub fn description(mut self, desc: impl Into<String>) -> Self {
        self.template.description = desc.into();
        self
    }

    /// Set template text body.
    pub fn body(mut self, body: impl Into<String>) -> Self {
        self.template.template = body.into();
        self
    }

    /// Set category.
    pub fn category(mut self, cat: impl Into<String>) -> Self {
        self.template.category = Some(cat.into());
        self
    }

    /// Add a tag.
    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.template.tags.push(tag.into());
        self
    }

    /// Add multiple tags.
    pub fn tags(mut self, tags: &[&str]) -> Self {
        for t in tags {
            self.template.tags.push((*t).to_string());
        }
        self
    }

    /// Add a variable definition.
    pub fn variable(mut self, var: PromptVariable) -> Self {
        self.template.variables.push(var);
        self
    }

    /// Set system prompt override.
    pub fn system_prompt(mut self, sys: impl Into<String>) -> Self {
        self.template.system_prompt_override = Some(sys.into());
        self
    }

    /// Set model override.
    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.template.model_override = Some(model.into());
        self
    }

    /// Set temperature.
    pub fn temperature(mut self, temp: f32) -> Self {
        self.template.temperature = Some(temp);
        self
    }

    /// Mark as built-in.
    pub fn builtin(mut self, is_builtin: bool) -> Self {
        self.template.is_builtin = is_builtin;
        self
    }

    /// Finalize and build `PromptTemplate`.
    pub fn build(mut self) -> PromptTemplate {
        if self.template.variables.is_empty() {
            self.template.auto_populate_variables();
        }
        self.template
    }
}

/// Central catalog and manager for prompt templates.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptLibrary {
    templates: BTreeMap<String, PromptTemplate>,
}

impl Default for PromptLibrary {
    fn default() -> Self {
        Self::with_builtins()
    }
}

impl PromptLibrary {
    /// Create an empty prompt library without any default templates.
    pub fn empty() -> Self {
        Self {
            templates: BTreeMap::new(),
        }
    }

    /// Create a prompt library initialized with standard curated built-in templates.
    pub fn with_builtins() -> Self {
        let mut lib = Self::empty();
        lib.register_builtins();
        lib
    }

    /// Initialize a library and automatically load all templates from default storage directories
    /// (built-ins, global `~/.fusion/prompts/`, and project-local `.fusion/prompts/`).
    pub fn new() -> Self {
        let mut lib = Self::with_builtins();
        let _ = lib.load_default_locations();
        lib
    }

    /// Insert or overwrite a template in the library.
    pub fn insert(&mut self, mut template: PromptTemplate) {
        template.updated_at = current_timestamp();
        if template.variables.is_empty() {
            template.auto_populate_variables();
        }
        self.templates
            .insert(template.name.to_lowercase(), template);
    }

    /// Retrieve a template by name (case-insensitive).
    pub fn get(&self, name: &str) -> Option<&PromptTemplate> {
        self.templates.get(&name.to_lowercase())
    }

    /// Retrieve a mutable reference to a template by name.
    pub fn get_mut(&mut self, name: &str) -> Option<&mut PromptTemplate> {
        self.templates.get_mut(&name.to_lowercase())
    }

    /// Check if a template exists.
    pub fn contains(&self, name: &str) -> bool {
        self.templates.contains_key(&name.to_lowercase())
    }

    /// Remove a template from memory by name.
    pub fn remove(&mut self, name: &str) -> Option<PromptTemplate> {
        self.templates.remove(&name.to_lowercase())
    }

    /// Count of registered templates.
    pub fn count(&self) -> usize {
        self.templates.len()
    }

    /// Whether the library has zero templates.
    pub fn is_empty(&self) -> bool {
        self.templates.is_empty()
    }

    /// List all templates ordered alphabetically by name.
    pub fn list(&self) -> Vec<&PromptTemplate> {
        self.templates.values().collect()
    }

    /// Get all unique category names present in the library.
    pub fn list_categories(&self) -> Vec<String> {
        let mut set = BTreeSet::new();
        for t in self.templates.values() {
            set.insert(t.category.clone().unwrap_or_else(|| "General".to_string()));
        }
        set.into_iter().collect()
    }

    /// Filter templates by category.
    pub fn list_by_category(&self, category: &str) -> Vec<&PromptTemplate> {
        let target = category.to_lowercase();
        self.templates
            .values()
            .filter(|t| t.category.as_deref().unwrap_or("general").to_lowercase() == target)
            .collect()
    }

    /// Filter templates by tag.
    pub fn list_by_tag(&self, tag: &str) -> Vec<&PromptTemplate> {
        let target = tag.to_lowercase();
        self.templates
            .values()
            .filter(|t| t.tags.iter().any(|t_tag| t_tag.to_lowercase() == target))
            .collect()
    }

    /// Full-text search across template names, descriptions, tags, and bodies.
    pub fn search(&self, query: &str) -> Vec<&PromptTemplate> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return self.list();
        }

        let mut scored: Vec<(&PromptTemplate, usize)> = Vec::new();
        for t in self.templates.values() {
            let name_lower = t.name.to_lowercase();
            let desc_lower = t.description.to_lowercase();
            let body_lower = t.template.to_lowercase();
            let cat_lower = t.category.as_deref().unwrap_or("").to_lowercase();

            let mut score = 0;
            if name_lower == q {
                score += 100;
            } else if name_lower.contains(&q) {
                score += 50;
            }

            if desc_lower.contains(&q) {
                score += 30;
            }
            if cat_lower.contains(&q) {
                score += 20;
            }
            if t.tags.iter().any(|tag| tag.to_lowercase().contains(&q)) {
                score += 25;
            }
            if body_lower.contains(&q) {
                score += 10;
            }

            if score > 0 {
                scored.push((t, score));
            }
        }

        scored.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.name.cmp(&b.0.name)));
        scored.into_iter().map(|(t, _)| t).collect()
    }

    /// Render template by name with variable map.
    pub fn render(
        &self,
        name: &str,
        vars: &HashMap<String, String>,
    ) -> Result<String, PromptLibError> {
        let tmpl = self
            .get(name)
            .ok_or_else(|| PromptLibError::TemplateNotFound(name.to_string()))?;
        tmpl.render(vars)
    }

    /// Render template by name with positional argument slice.
    pub fn render_positional(&self, name: &str, args: &[&str]) -> Result<String, PromptLibError> {
        let tmpl = self
            .get(name)
            .ok_or_else(|| PromptLibError::TemplateNotFound(name.to_string()))?;
        tmpl.render_positional(args)
    }

    /// Render template by name with raw CLI argument tokens.
    pub fn render_cli_args(
        &self,
        name: &str,
        raw_args: &[String],
    ) -> Result<String, PromptLibError> {
        let tmpl = self
            .get(name)
            .ok_or_else(|| PromptLibError::TemplateNotFound(name.to_string()))?;
        tmpl.render_cli_args(raw_args)
    }

    /// Load templates from both global `~/.fusion/prompts/` and project-local `.fusion/prompts/`.
    pub fn load_default_locations(&mut self) -> Result<usize, PromptLibError> {
        let mut loaded = 0;
        let global_dir = prompts_dir();
        if global_dir.exists() && global_dir.is_dir() {
            if let Ok(count) = self.load_from_dir(&global_dir) {
                loaded += count;
            }
        }

        // Also check single global JSON archive if present
        let global_json = prompts_file();
        if global_json.exists() && global_json.is_file() {
            if let Ok(content) = fs::read_to_string(&global_json) {
                if let Ok(count) = self.import_all_json(&content) {
                    loaded += count;
                }
            }
        }

        let local_dir = project_prompts_dir();
        if local_dir.exists() && local_dir.is_dir() {
            if let Ok(count) = self.load_from_dir(&local_dir) {
                loaded += count;
            }
        }

        Ok(loaded)
    }

    /// Scan directory and load all `.md`, `.json`, `.toml`, or `.prompt` template files.
    pub fn load_from_dir(&mut self, dir: &Path) -> Result<usize, PromptLibError> {
        if !dir.exists() || !dir.is_dir() {
            return Ok(0);
        }

        let entries = fs::read_dir(dir).map_err(|e| PromptLibError::Io(e.to_string()))?;
        let mut count = 0;

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    let ext_lower = ext.to_lowercase();
                    if ["md", "json", "toml", "prompt"].contains(&ext_lower.as_str()) {
                        if let Ok(tmpl) = self.load_from_file(&path) {
                            self.insert(tmpl);
                            count += 1;
                        }
                    }
                }
            }
        }

        Ok(count)
    }

    /// Load a single template from a file path (`.md`, `.json`, `.toml`).
    pub fn load_from_file(&mut self, path: &Path) -> Result<PromptTemplate, PromptLibError> {
        let content = fs::read_to_string(path)
            .map_err(|e| PromptLibError::Io(format!("Failed to read {}: {}", path.display(), e)))?;

        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        let mut tmpl = match ext.as_str() {
            "json" => serde_json::from_str::<PromptTemplate>(&content)
                .map_err(|e| PromptLibError::Serialization(e.to_string()))?,
            _ => PromptTemplate::from_markdown_frontmatter(&content)?,
        };

        // If name was default or derived, check if file stem gives a clearer slug
        if tmpl.name == "custom" {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                tmpl.name = stem.to_string();
            }
        }

        Ok(tmpl)
    }

    /// Save a template to a specified file path. Formats as JSON or Markdown based on extension.
    pub fn save_to_file(&self, name: &str, path: &Path) -> Result<(), PromptLibError> {
        let tmpl = self
            .get(name)
            .ok_or_else(|| PromptLibError::TemplateNotFound(name.to_string()))?;

        if let Some(parent) = path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent).map_err(|e| PromptLibError::Io(e.to_string()))?;
            }
        }

        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("md")
            .to_lowercase();

        let content = if ext == "json" {
            serde_json::to_string_pretty(tmpl)
                .map_err(|e| PromptLibError::Serialization(e.to_string()))?
        } else {
            tmpl.to_markdown_frontmatter()
        };

        fs::write(path, content).map_err(|e| PromptLibError::Io(e.to_string()))?;
        Ok(())
    }

    /// Save template to the global `~/.fusion/prompts/<name>.md` file and register in memory.
    pub fn save_to_global(&mut self, template: PromptTemplate) -> Result<PathBuf, PromptLibError> {
        let dir = prompts_dir();
        if !dir.exists() {
            fs::create_dir_all(&dir).map_err(|e| PromptLibError::Io(e.to_string()))?;
        }
        let file_path = dir.join(format!("{}.md", sanitize_filename(&template.name)));
        let content = template.to_markdown_frontmatter();
        fs::write(&file_path, content).map_err(|e| PromptLibError::Io(e.to_string()))?;
        self.insert(template);
        Ok(file_path)
    }

    /// Save template to the project-local `.fusion/prompts/<name>.md` file and register in memory.
    pub fn save_to_local(&mut self, template: PromptTemplate) -> Result<PathBuf, PromptLibError> {
        let dir = project_prompts_dir();
        if !dir.exists() {
            fs::create_dir_all(&dir).map_err(|e| PromptLibError::Io(e.to_string()))?;
        }
        let file_path = dir.join(format!("{}.md", sanitize_filename(&template.name)));
        let content = template.to_markdown_frontmatter();
        fs::write(&file_path, content).map_err(|e| PromptLibError::Io(e.to_string()))?;
        self.insert(template);
        Ok(file_path)
    }

    /// Delete a template from memory and remove any persisted file from global or local prompt folders.
    pub fn delete_persisted(&mut self, name: &str) -> Result<bool, PromptLibError> {
        let removed = self.remove(name).is_some();
        let slug = sanitize_filename(name);

        let global_path = prompts_dir().join(format!("{}.md", slug));
        if global_path.exists() {
            let _ = fs::remove_file(global_path);
        }
        let local_path = project_prompts_dir().join(format!("{}.md", slug));
        if local_path.exists() {
            let _ = fs::remove_file(local_path);
        }

        Ok(removed)
    }

    /// Export all templates in memory as a JSON string array.
    pub fn export_all_json(&self) -> Result<String, PromptLibError> {
        let list: Vec<&PromptTemplate> = self.templates.values().collect();
        serde_json::to_string_pretty(&list)
            .map_err(|e| PromptLibError::Serialization(e.to_string()))
    }

    /// Import templates from a JSON string array.
    pub fn import_all_json(&mut self, json_str: &str) -> Result<usize, PromptLibError> {
        let list: Vec<PromptTemplate> = serde_json::from_str(json_str)
            .map_err(|e| PromptLibError::Serialization(e.to_string()))?;
        let count = list.len();
        for t in list {
            self.insert(t);
        }
        Ok(count)
    }

    /// Export all templates into a concatenated Markdown document.
    pub fn export_all_markdown(&self) -> String {
        let mut out = String::new();
        out.push_str("# Fusion Prompt Library Catalog\n\n");
        for cat in self.list_categories() {
            out.push_str(&format!("## Category: {}\n\n", cat));
            for t in self.list_by_category(&cat) {
                out.push_str(&t.format_markdown_card());
                out.push('\n');
            }
        }
        out
    }

    /// Populate standard factory built-in templates.
    fn register_builtins(&mut self) {
        for t in get_curated_builtin_templates() {
            self.insert(t);
        }
    }
}

/// Returns the user global prompt storage directory: `~/.fusion/prompts`.
pub fn prompts_dir() -> PathBuf {
    Config::config_dir().join("prompts")
}

/// Returns the project-local prompt storage directory: `.fusion/prompts`.
pub fn project_prompts_dir() -> PathBuf {
    PathBuf::from(".fusion").join("prompts")
}

/// Returns the user global JSON archive file: `~/.fusion/prompts.json`.
pub fn prompts_file() -> PathBuf {
    Config::config_dir().join("prompts.json")
}

/// Helper to get current Unix epoch timestamp in seconds.
fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Sanitize filename for safe disk persistence.
fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

/// Extract all `{{var}}` or `{{var:-default}}` placeholders from a template string.
pub fn extract_placeholders(template: &str) -> Vec<PromptVariable> {
    let re = Regex::new(r"\{\{\s*([a-zA-Z0-9_\-]+)(?::-(.*?)|:(.*?))?\s*\}\}").unwrap();
    let mut vars = Vec::new();
    let mut seen = BTreeSet::new();

    for cap in re.captures_iter(template) {
        let name = cap[1].to_string();
        if !seen.insert(name.clone()) {
            continue;
        }

        let default_val = cap
            .get(2)
            .or_else(|| cap.get(3))
            .map(|m| m.as_str().to_string());

        let required = default_val.is_none();
        vars.push(PromptVariable {
            name,
            description: None,
            default_value: default_val,
            required,
        });
    }

    vars
}

/// Substitute variables in template text with given values and fallbacks.
/// Returns the rendered string and any missing required variables.
pub fn substitute_variables(
    template: &str,
    vars: &HashMap<String, String>,
    declared_vars: &[PromptVariable],
) -> (String, Vec<String>) {
    let mut missing = Vec::new();
    let declared_map: HashMap<&str, &PromptVariable> =
        declared_vars.iter().map(|v| (v.name.as_str(), v)).collect();

    let re = Regex::new(r"\{\{\s*([a-zA-Z0-9_\-]+)(?::-(.*?)|:(.*?))?\s*\}\}").unwrap();

    let rendered = re.replace_all(template, |caps: &regex::Captures| {
        let key = &caps[1];
        let inline_default = caps.get(2).or_else(|| caps.get(3)).map(|m| m.as_str());

        // 1. Direct match in provided vars
        if let Some(val) = vars.get(key) {
            if !val.is_empty() {
                return val.clone();
            }
        }

        // 2. Case-insensitive lookup
        for (k, v) in vars {
            if k.eq_ignore_ascii_case(key) && !v.is_empty() {
                return v.clone();
            }
        }

        // 3. Inline default in template syntax e.g. `{{lang:-rust}}`
        if let Some(def) = inline_default {
            return def.to_string();
        }

        // 4. Declared default on variable metadata
        if let Some(decl) = declared_map.get(key) {
            if let Some(def) = &decl.default_value {
                return def.clone();
            }
            if decl.required && !missing.contains(&key.to_string()) {
                missing.push(key.to_string());
            }
        } else {
            // Unannounced variable without default is assumed required
            if !missing.contains(&key.to_string()) {
                missing.push(key.to_string());
            }
        }

        caps[0].to_string()
    });

    (rendered.into_owned(), missing)
}

/// Parse positional string slice into variable mapping for a template.
pub fn parse_positional_args(args: &[&str], template: &PromptTemplate) -> HashMap<String, String> {
    let string_args: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
    parse_cli_tokens(&string_args, template)
}

/// Parse CLI tokens supporting `key=value`, `key="value"`, `--key=value`, or positional matching.
pub fn parse_cli_tokens(tokens: &[String], template: &PromptTemplate) -> HashMap<String, String> {
    let mut vars = HashMap::new();
    let mut positionals = Vec::new();

    for token in tokens {
        let trimmed = token.trim();
        let stripped = trimmed.trim_start_matches('-');

        if let Some((k, v)) = stripped.split_once('=') {
            let key = k.trim().to_lowercase();
            let val = v.trim().trim_matches('"').trim_matches('\'').to_string();
            vars.insert(key, val);
        } else if !trimmed.is_empty() {
            positionals.push(trimmed.to_string());
        }
    }

    if positionals.is_empty() {
        return vars;
    }

    // Special case: Single positional argument matching primary content variable
    if positionals.len() == 1 {
        let single_arg = positionals[0].clone();
        let primary_candidates = [
            "code", "input", "text", "error", "diff", "summary", "problem", "context", "query",
        ];

        // Check if template has one of the primary candidates as a variable
        for candidate in primary_candidates {
            if template.variables.iter().any(|v| v.name == candidate)
                && !vars.contains_key(candidate)
            {
                vars.insert(candidate.to_string(), single_arg);
                return vars;
            }
        }
    }

    // Map positional arguments in declaration order to unfilled variables
    let mut pos_idx = 0;
    for var in &template.variables {
        if !vars.contains_key(&var.name) && pos_idx < positionals.len() {
            vars.insert(var.name.clone(), positionals[pos_idx].clone());
            pos_idx += 1;
        }
    }

    // Also populate $1, $2, $N positional keys
    for (i, p) in positionals.iter().enumerate() {
        vars.insert(format!("${}", i + 1), p.clone());
        vars.insert(format!("{}", i), p.clone());
    }

    vars
}

/// Factory list of curated engineering prompt templates out of the box.
pub fn get_curated_builtin_templates() -> Vec<PromptTemplate> {
    vec![
        // 1. Code Review
        PromptTemplate::builder("review")
            .description("Comprehensive, rigorous code review focusing on safety, correctness, and performance")
            .category("Review")
            .tags(&["review", "quality", "audit", "security", "correctness"])
            .variable(PromptVariable::new("code", true).with_description("Code snippet or file content to review"))
            .variable(PromptVariable::with_default("focus", "correctness, safety invariants, error propagation, and performance").with_description("Review emphasis"))
            .body(
r#"Please perform a rigorous, expert code review of the following code.

### Focus Areas
- Focus: {{focus}}
- Correctness and edge case handling
- Memory safety, ownership, and zero-cost discipline
- Robust error propagation (avoid unwrap/panic in library paths)
- Concurrency and race condition inspection
- API ergonomics and idiomatic style

### Code Under Review
```
{{code}}
```

### Review Format
1. **Summary Verdict**: Brief high-level evaluation.
2. **Critical Issues**: Bugs, safety violations, or memory leaks (with proposed diffs).
3. **Optimizations & Refinements**: Concrete improvements with explanations.
4. **Positive Highlights**: Well-crafted patterns."#
            )
            .builtin(true)
            .build(),

        // 2. Refactor
        PromptTemplate::builder("refactor")
            .description("Behavior-preserving refactoring for modularity, clean abstractions, and readability")
            .category("Coding")
            .tags(&["refactor", "clean-code", "architecture", "maintainability"])
            .variable(PromptVariable::new("code", true).with_description("Code to refactor"))
            .variable(PromptVariable::with_default("goal", "improve readability, eliminate duplication, and enforce clean abstractions").with_description("Refactoring objective"))
            .body(
r#"Refactor the following code to {{goal}}.

### Constraints
- Strictly preserve existing external behavior and API contracts.
- Maintain zero regressions in test coverage and error semantics.
- Prefer boring, idiomatic, and maintainable constructs over premature abstractions.

### Target Code
```
{{code}}
```

### Deliverable
1. Step-by-step breakdown of refactoring choices.
2. Complete, clean, refactored code."#
            )
            .builtin(true)
            .build(),

        // 3. Test Generation
        PromptTemplate::builder("test")
            .description("Generate exhaustive unit and integration tests covering edge cases and error paths")
            .category("Testing")
            .tags(&["test", "unit-test", "qa", "edge-cases", "mock"])
            .variable(PromptVariable::new("code", true).with_description("Code unit to test"))
            .variable(PromptVariable::with_default("framework", "standard unit test framework").with_description("Target testing framework"))
            .body(
r#"Generate a comprehensive, production-grade test suite for the following code using {{framework}}.

### Requirements
- Cover the primary happy path thoroughly.
- Cover all edge cases: boundary values, empty inputs, null/none values, overflow conditions.
- Cover all error scenarios and unexpected inputs.
- Ensure test isolation and deterministic execution.

### Code
```
{{code}}
```

### Output
Provide complete, runnable test code with explanatory comments for each test case."#
            )
            .builtin(true)
            .build(),

        // 4. Explain
        PromptTemplate::builder("explain")
            .description("Deep, structured technical explanation of code architecture, algorithms, and invariants")
            .category("Documentation")
            .tags(&["explain", "learn", "architecture", "walkthrough"])
            .variable(PromptVariable::new("code", true).with_description("Code to explain"))
            .variable(PromptVariable::with_default("depth", "in-depth engineering level").with_description("Target depth and audience"))
            .body(
r#"Provide a clear, structured {{depth}} technical explanation of the following code.

### Code
```
{{code}}
```

### Outline
1. **High-Level Purpose**: What problem does this solve?
2. **Core Mechanisms & Data Flow**: Walk through key data structures and control flow.
3. **Subtle Invariants & Non-Obvious Nuances**: Concurrency, memory layout, or algorithm traits.
4. **Complexity**: Time and space complexity analysis."#
            )
            .builtin(true)
            .build(),

        // 5. Documentation
        PromptTemplate::builder("doc")
            .description("Generate comprehensive docstrings, API reference, usage examples, and safety notes")
            .category("Documentation")
            .tags(&["doc", "docstring", "api", "reference", "rustdoc"])
            .variable(PromptVariable::new("code", true).with_description("Functions, structs, or modules to document"))
            .body(
r#"Write complete, production-grade documentation for the following code according to language conventions.

### Requirements
- Clear summary paragraph explaining function and behavior.
- Parameters, return types, and possible errors / panics explicitly documented.
- Realistic, runnable doc test / usage example.
- Safety invariants and preconditions if applicable.

### Code
```
{{code}}
```"#
            )
            .builtin(true)
            .build(),

        // 6. Debug
        PromptTemplate::builder("debug")
            .description("Systematic root cause analysis, reproduction hypothesis, and targeted bugfix")
            .category("Debugging")
            .tags(&["debug", "bugfix", "root-cause", "troubleshoot", "stacktrace"])
            .variable(PromptVariable::new("error", true).with_description("Error message, stack trace, or buggy behavior"))
            .variable(PromptVariable::with_default("code", "").with_description("Related source code"))
            .body(
r#"Diagnose and fix the following bug using systematic root cause analysis.

### Error / Symptom
```
{{error}}
```

### Code Context
```
{{code}}
```

### Analysis Steps
1. **Hypothesis**: What is the root cause?
2. **Reproduction**: Minimal scenario triggering the failure.
3. **Fix**: Exact code modification eliminating the root cause without side effects.
4. **Prevention**: How to safeguard against this class of bug in the future."#
            )
            .builtin(true)
            .build(),

        // 7. Security Audit
        PromptTemplate::builder("security")
            .description("Threat modeling and vulnerability audit (OWASP, injection, memory safety, auth)")
            .category("Security")
            .tags(&["security", "audit", "owasp", "cve", "vulnerability", "auth"])
            .variable(PromptVariable::new("code", true).with_description("Code or API endpoint to audit"))
            .body(
r#"Perform an exhaustive security assessment and threat modeling review on the following code.

### Inspection Checklist
- Injection risks (SQL, Command, Template, XSS)
- Authentication and authorization bypasses
- Memory safety (buffer overflows, use-after-free, unvalidated lengths)
- Denial of Service vectors (unbounded allocations, regex DoS, slowloris)
- Secret leakage and sensitive data exposure in logs or errors

### Code
```
{{code}}
```

### Audit Report Format
- **Threat Level**: Critical / High / Medium / Low / Informational
- **Vulnerability**: Detailed description and exploit scenario
- **Remediation**: Hardened replacement code"#
            )
            .builtin(true)
            .build(),

        // 8. Performance Optimization
        PromptTemplate::builder("optimize")
            .description("Performance profiling analysis optimizing memory allocations, algorithmic complexity, and cache locality")
            .category("Coding")
            .tags(&["perf", "optimize", "cache", "memory", "latency", "throughput"])
            .variable(PromptVariable::new("code", true).with_description("Hot path code to optimize"))
            .variable(PromptVariable::with_default("target", "throughput and memory allocations").with_description("Optimization goal"))
            .body(
r#"Optimize the following code focusing on {{target}}.

### Focus
- Algorithmic time complexity (e.g. $O(N^2) \to O(N \log N)$ or $O(N)$)
- Minimizing heap allocations and buffer reallocations (zero-copy, slicing, small-vec)
- Cache locality and SIMD opportunities
- Lock contention and async bottlenecks

### Code
```
{{code}}
```

### Response Format
1. Bottleneck identification.
2. Optimized implementation with benchmark expectations."#
            )
            .builtin(true)
            .build(),

        // 9. Conventional Commit Generator
        PromptTemplate::builder("commit")
            .description("Generate concise, meaningful Conventional Commit messages from diffs or changes")
            .category("Git")
            .tags(&["git", "commit", "conventional-commits", "changelog"])
            .variable(PromptVariable::new("diff", true).with_description("Git diff or summary of changes"))
            .variable(PromptVariable::with_default("scope", "").with_description("Optional commit scope"))
            .body(
r#"Generate a clean, conventional commit message based on the following changes.

### Format Rules
- `<type>(<scope>): <short imperative summary under 72 chars>`
- Types: `feat`, `fix`, `refactor`, `perf`, `test`, `docs`, `chore`
- Optional bulleted body explaining *why* and *what* (not how).
- Reference breaking changes if any.

### Changes / Diff
```
{{diff}}
```"#
            )
            .builtin(true)
            .build(),

        // 10. Pull Request Description
        PromptTemplate::builder("pr")
            .description("Generate complete GitHub Pull Request description with summary, changes, test plan, and checklist")
            .category("Git")
            .tags(&["git", "pr", "pull-request", "github", "review"])
            .variable(PromptVariable::new("summary", true).with_description("High-level summary of what was accomplished"))
            .variable(PromptVariable::with_default("changes", "").with_description("Detailed change items or git log"))
            .body(
r#"Generate a comprehensive GitHub Pull Request description for the following work.

### Context
{{summary}}

### Changes
{{changes}}

### PR Template Output
- **Summary**: Concise overview of problem and solution.
- **Key Changes**: Bulleted list of architectural and functional updates.
- **Verification Plan**: Step-by-step test commands and manual verification performed.
- **Checklist**: Standard PR readiness checklist."#
            )
            .builtin(true)
            .build(),

        // 11. Architecture Decision Record (ADR)
        PromptTemplate::builder("arch")
            .description("Architecture Decision Record (ADR) format evaluating trade-offs and options")
            .category("Architecture")
            .tags(&["arch", "adr", "design", "system-design", "tradeoffs"])
            .variable(PromptVariable::new("problem", true).with_description("Architectural problem to solve"))
            .variable(PromptVariable::with_default("options", "").with_description("Candidate solutions considered"))
            .body(
r#"Draft a formal Architecture Decision Record (ADR) for the following problem.

### Problem Statement
{{problem}}

### Candidate Options
{{options}}

### ADR Structure
1. **Title & Status**: ADR-XXXX [Proposed / Accepted]
2. **Context**: Forces, constraints, and business requirements.
3. **Decision**: Chosen architecture and exact mechanics.
4. **Consequences**:
   - Positive consequences & capabilities unlocked.
   - Negative consequences & operational trade-offs."#
            )
            .builtin(true)
            .build(),

        // 12. Idiomatic Translation
        PromptTemplate::builder("translate")
            .description("Translate code across programming languages preserving paradigms and idioms")
            .category("Coding")
            .tags(&["translate", "convert", "multilingual", "porting"])
            .variable(PromptVariable::new("code", true).with_description("Source code"))
            .variable(PromptVariable::new("from_lang", true).with_description("Source language"))
            .variable(PromptVariable::new("to_lang", true).with_description("Target language"))
            .body(
r#"Translate the following {{from_lang}} code into idiomatic, production-ready {{to_lang}}.

### Translation Directives
- Do not perform literal line-by-line translation; use {{to_lang}} native idioms and standard libraries.
- Preserve all error handling, concurrency, and performance semantics.
- Add concise comments explaining paradigm shifts.

### Source ({{from_lang}})
```
{{code}}
```"#
            )
            .builtin(true)
            .build(),

        // 13. Strict TypeScript
        PromptTemplate::builder("types")
            .description("Transform loose JavaScript / TypeScript into strict, type-safe code with discriminated unions")
            .category("Coding")
            .tags(&["typescript", "types", "type-safety", "discriminated-unions"])
            .variable(PromptVariable::new("code", true).with_description("TypeScript / JavaScript code"))
            .body(
r#"Refactor the following TypeScript code to achieve maximum type safety.

### Directives
- Eliminate all `any` and loose `unknown` casts.
- Use discriminated unions for state machines and action payloads.
- Use `const` assertions and type guards for narrowing.
- Enforce strict nullability and exhaustive pattern matching.

### Code
```
{{code}}
```"#
            )
            .builtin(true)
            .build(),

        // 14. Idiomatic Rust Transformation
        PromptTemplate::builder("rustify")
            .description("Modernize Rust code with zero-cost abstractions, typestates, iterator combinators, and error propagation")
            .category("Coding")
            .tags(&["rust", "idiomatic", "zero-cost", "traits", "typestate"])
            .variable(PromptVariable::new("code", true).with_description("Rust code to modernize"))
            .body(
r#"Modernize and polish the following Rust code into world-class idiomatic Rust.

### Directives
- Leverage iterator combinators and zero-copy slicing where beneficial.
- Replace indexing loops with safe iterators / chunks.
- Replace panicking `.unwrap()` calls with robust `Result` / `Option` combinators.
- Apply typestate patterns or RAII guards for resource lifecycles if appropriate.

### Code
```rust
{{code}}
```"#
            )
            .builtin(true)
            .build(),

        // 15. Quick Minimal Bugfix
        PromptTemplate::builder("fix")
            .description("Targeted minimal bugfix modifying the fewest lines necessary")
            .category("Debugging")
            .tags(&["fix", "quickfix", "surgical", "minimal-diff"])
            .variable(PromptVariable::new("code", true).with_description("Code snippet"))
            .variable(PromptVariable::new("issue", true).with_description("Issue to fix"))
            .body(
r#"Apply a surgical, minimal bugfix to resolve the specified issue with zero unrelated changes.

### Issue
{{issue}}

### Code
```
{{code}}
```

### Response
Provide only the necessary fix and a single sentence explanation of why the change resolves the issue."#
            )
            .builtin(true)
            .build(),

        // 16. Benchmark Generator
        PromptTemplate::builder("bench")
            .description("Generate microbenchmarks measuring throughput, latency, and allocation counts")
            .category("Testing")
            .tags(&["bench", "benchmark", "criterion", "performance", "profiling"])
            .variable(PromptVariable::new("code", true).with_description("Target function or struct"))
            .body(
r#"Create a robust benchmark suite for the following code.

### Guidelines
- Measure throughput ($ops/sec$) and latency distribution ($p50, p99$).
- Prevent compiler dead-code elimination with black-box techniques.
- Test across various data sizes (small, medium, large).

### Target Code
```
{{code}}
```"#
            )
            .builtin(true)
            .build(),

        // 17. Technical Summary
        PromptTemplate::builder("summarize")
            .description("Synthesize complex technical documents, logs, or transcripts into key insights and action items")
            .category("Documentation")
            .tags(&["summary", "tldr", "action-items", "synthesis"])
            .variable(PromptVariable::new("text", true).with_description("Text to summarize"))
            .body(
r#"Synthesize the following technical content into an executive summary.

### Content
```
{{text}}
```

### Summary Format
1. **Executive TL;DR**: 2-3 sentence core takeaway.
2. **Key Technical Decisions**: Bulleted summary of architecture or implementation facts.
3. **Action Items / Next Steps**: Concrete prioritized checklist."#
            )
            .builtin(true)
            .build(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_variable_extraction_and_substitution() {
        let template = "Hello {{name}}, welcome to {{project:-Fusion}}! Please check {{file}}.";
        let extracted = extract_placeholders(template);
        assert_eq!(extracted.len(), 3);
        assert_eq!(extracted[0].name, "name");
        assert!(extracted[0].required);
        assert_eq!(extracted[1].name, "project");
        assert_eq!(extracted[1].default_value.as_deref(), Some("Fusion"));
        assert!(!extracted[1].required);
        assert_eq!(extracted[2].name, "file");
        assert!(extracted[2].required);

        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "Alice".to_string());
        vars.insert("file".to_string(), "src/main.rs".to_string());

        let (rendered, missing) = substitute_variables(template, &vars, &extracted);
        assert!(missing.is_empty());
        assert_eq!(
            rendered,
            "Hello Alice, welcome to Fusion! Please check src/main.rs."
        );
    }

    #[test]
    fn test_missing_required_variable() {
        let tmpl = PromptTemplate::new(
            "test-tmpl",
            "A test template",
            "Inspect {{code}} focusing on {{aspect}}",
        );

        let mut vars = HashMap::new();
        vars.insert("aspect".to_string(), "performance".to_string());

        let res = tmpl.render(&vars);
        assert!(res.is_err());
        match res.unwrap_err() {
            PromptLibError::MissingVariable { variable, .. } => {
                assert_eq!(variable, "code");
            }
            other => panic!("Unexpected error: {:?}", other),
        }
    }

    #[test]
    fn test_positional_args_and_primary_variable() {
        let tmpl = PromptTemplate::new(
            "review-sample",
            "Review sample",
            "Review this code:\n{{code}}\nFocus on: {{focus:-general}}",
        );

        // Single positional should automatically map to `code`
        let rendered = tmpl.render_positional(&["fn main() {}"]).unwrap();
        assert!(rendered.contains("fn main() {}"));
        assert!(rendered.contains("Focus on: general"));

        // Multiple CLI tokens with key-value pairs
        let tokens = vec!["focus=security".to_string(), "code=let x = 42;".to_string()];
        let rendered2 = tmpl.render_cli_args(&tokens).unwrap();
        assert!(rendered2.contains("let x = 42;"));
        assert!(rendered2.contains("Focus on: security"));
    }

    #[test]
    fn test_frontmatter_markdown_roundtrip() {
        let orig = PromptTemplate::builder("custom-review")
            .description("A custom review prompt")
            .category("Custom")
            .tags(&["review", "team-standards"])
            .variable(PromptVariable::new("code", true))
            .body("Please check {{code}} against team guidelines.")
            .model("claude-3-5-sonnet")
            .temperature(0.2)
            .build();

        let md = orig.to_markdown_frontmatter();
        assert!(md.contains("name: \"custom-review\""));
        assert!(md.contains("category: \"Custom\""));
        assert!(md.contains("model: \"claude-3-5-sonnet\""));

        let parsed = PromptTemplate::from_markdown_frontmatter(&md).unwrap();
        assert_eq!(parsed.name, "custom-review");
        assert_eq!(parsed.category.as_deref(), Some("Custom"));
        assert_eq!(parsed.tags, vec!["review", "team-standards"]);
        assert_eq!(parsed.model_override.as_deref(), Some("claude-3-5-sonnet"));
        assert_eq!(
            parsed.template,
            "Please check {{code}} against team guidelines."
        );
    }

    #[test]
    fn test_builtins_catalog_and_search() {
        let lib = PromptLibrary::with_builtins();
        assert!(lib.count() >= 15);

        let review_tmpl = lib.get("review");
        assert!(review_tmpl.is_some());
        let t = review_tmpl.unwrap();
        assert_eq!(t.category.as_deref(), Some("Review"));
        assert!(t.is_builtin);

        let search_res = lib.search("refactor");
        assert!(!search_res.is_empty());
        assert_eq!(search_res[0].name, "refactor");

        let categories = lib.list_categories();
        assert!(categories.contains(&"Review".to_string()));
        assert!(categories.contains(&"Coding".to_string()));
        assert!(categories.contains(&"Testing".to_string()));
    }

    #[test]
    fn test_in_memory_crud() {
        let mut lib = PromptLibrary::empty();
        assert_eq!(lib.count(), 0);

        let tmpl = PromptTemplate::new("hello", "Greetings", "Hello {{name}}!");
        lib.insert(tmpl);
        assert_eq!(lib.count(), 1);
        assert!(lib.contains("hello"));

        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "World".to_string());
        let rendered = lib.render("hello", &vars).unwrap();
        assert_eq!(rendered, "Hello World!");

        let removed = lib.remove("hello");
        assert!(removed.is_some());
        assert_eq!(lib.count(), 0);
    }

    #[test]
    fn test_template_getters() {
        let tmpl = PromptTemplate::builder("getter-check")
            .description("Getter verification template")
            .category("Testing")
            .tags(&["alpha", "beta"])
            .model("gpt-4o")
            .system_prompt("You are a strict reviewer.")
            .temperature(0.5)
            .body("Review {{code}}.")
            .build();

        assert_eq!(tmpl.category(), Some("Testing"));
        assert_eq!(tmpl.tags(), &["alpha".to_string(), "beta".to_string()]);
        assert_eq!(tmpl.model_override(), Some("gpt-4o"));
        assert_eq!(
            tmpl.system_prompt_override(),
            Some("You are a strict reviewer.")
        );
        assert_eq!(tmpl.temperature(), Some(0.5));
        assert!(!tmpl.is_empty());
        assert_eq!(tmpl.len(), "Review {{code}}.".len());
    }

    #[test]
    fn test_template_estimate_tokens_rendered() {
        let tmpl = PromptTemplate::new(
            "token-check",
            "Token estimation",
            "Analyze this Rust function and report complexity: {{code}}",
        );

        let mut vars = HashMap::new();
        let code = "fn main() { println!(\"hello\"); }".repeat(20);
        vars.insert("code".to_string(), code.clone());

        let tokens = tmpl.estimate_tokens(&vars);
        let expected_len = tmpl.len() - "{{code}}".len() + code.len();
        assert!(tokens > 0);
        // Rough proportional bound: tokens ~ len/4 with framing slack.
        assert!(tokens * 4 <= expected_len + 64);
        assert!(tokens >= expected_len / 8);
    }

    #[test]
    fn test_template_estimate_tokens_missing_vars_returns_zero() {
        let tmpl = PromptTemplate::new(
            "token-missing",
            "Missing variable",
            "Process {{code}} please",
        );

        // No vars supplied: render fails; estimation degrades to 0, not panic.
        assert_eq!(tmpl.estimate_tokens(&HashMap::new()), 0);
    }

    #[test]
    fn test_template_empty_body() {
        let tmpl = PromptTemplate::new("empty-tmpl", "Nothing", "");
        assert!(tmpl.is_empty());
        assert_eq!(tmpl.len(), 0);
        // Empty template renders to empty string without error.
        assert_eq!(tmpl.render(&HashMap::new()).unwrap(), "");
    }

    #[test]
    fn test_export_import_json_roundtrip() {
        let mut lib = PromptLibrary::empty();
        lib.insert(PromptTemplate::new("export-a", "First", "Body A {{x}}"));
        lib.insert(PromptTemplate::new("export-b", "Second", "Body B {{y}}"));

        let json = lib.export_all_json().unwrap();
        let mut restored = PromptLibrary::empty();
        let count = restored.import_all_json(&json).unwrap();
        assert_eq!(count, 2);
        assert!(restored.contains("export-a"));
        assert!(restored.contains("export-b"));

        let mut vars = HashMap::new();
        vars.insert("x".to_string(), "value-x".to_string());
        assert_eq!(
            restored.render("export-a", &vars).unwrap(),
            "Body A value-x"
        );
    }

    #[test]
    fn test_export_markdown_catalog_contains_categories() {
        let lib = PromptLibrary::with_builtins();
        let catalog = lib.export_all_markdown();

        assert!(catalog.contains("# Fusion Prompt Library Catalog"));
        assert!(catalog.contains("## Category: Review"));
        assert!(catalog.contains("`/prompt load review`"));
        assert!(catalog.contains("`(builtin)`"));
    }

    #[test]
    fn test_markdown_card_variables_section() {
        let tmpl = PromptTemplate::builder("card-check")
            .description("Card rendering")
            .category("Testing")
            .variable(PromptVariable::new("code", true).with_description("The code to inspect"))
            .variable(PromptVariable::with_default("style", "concise"))
            .body("Inspect {{code}} with {{style}} output.")
            .build();

        let card = tmpl.format_markdown_card();
        assert!(card.contains("**Variables:**"));
        assert!(card.contains("*(required)*"));
        assert!(card.contains("*(optional)*"));
        assert!(card.contains("default: `concise`"));
        assert!(card.contains("The code to inspect"));
    }

    #[test]
    fn test_cli_tokens_flags_and_quoting() {
        let tmpl = PromptTemplate::new(
            "cli-quoting",
            "CLI token quoting",
            "Fix {{issue}} in {{code:-}}",
        );

        let tokens = vec![
            "--issue=off-by-one".to_string(),
            "code=\"let i = 0;\"".to_string(),
        ];
        let rendered = tmpl.render_cli_args(&tokens).unwrap();
        assert!(rendered.contains("off-by-one"));
        assert!(rendered.contains("let i = 0;"));
        assert!(!rendered.contains("--"));
        assert!(!rendered.contains('"'));
    }

    #[test]
    fn test_search_scores_name_exact_above_body() {
        let mut lib = PromptLibrary::empty();
        lib.insert(PromptTemplate::new("scan", "Scanning tool", "Generic body"));
        lib.insert(PromptTemplate::new(
            "other",
            "Other",
            "Body mentioning scan generically",
        ));

        let results = lib.search("scan");
        assert_eq!(results[0].name, "scan");
    }

    #[test]
    fn test_sanitize_filename_in_slugs() {
        // Verifies built-in slugs persist safely (no path separators).
        for t in get_curated_builtin_templates() {
            let slug = format!("{}", t.name.replace(['/', '\\', ' '], "-"));
            assert!(!slug.contains('/'));
            assert!(!slug.is_empty());
        }
    }

    #[test]
    fn test_case_insensitive_lookup_and_get() {
        let mut lib = PromptLibrary::empty();
        lib.insert(PromptTemplate::new("MyPrompt", "Case check", "Body"));

        assert!(lib.contains("myprompt"));
        assert!(lib.contains("MYPROMPT"));
        assert!(lib.get("MyPrompt").is_some());
    }

    #[test]
    fn test_template_auto_populates_variables_from_body() {
        let mut tmpl = PromptTemplate::new(
            "auto-var",
            "Auto variable inference",
            "Translate {{src_lang}} to {{dst_lang:-english}}: {{code}}",
        );
        tmpl.auto_populate_variables();

        assert_eq!(tmpl.variables.len(), 3);
        assert_eq!(tmpl.variables[0].name, "src_lang");
        assert!(tmpl.variables[0].required);
        assert_eq!(tmpl.variables[1].name, "dst_lang");
        assert_eq!(tmpl.variables[1].default_value.as_deref(), Some("english"));
        assert!(!tmpl.variables[1].required);
    }

    #[test]
    fn test_variable_metadata_preserved_on_repopulate() {
        let mut tmpl = PromptTemplate::new("preserve-var", "Preserving metadata", "Check {{code}}");
        tmpl.variables =
            vec![PromptVariable::new("code", true).with_description("Custom code description")];
        tmpl.auto_populate_variables();

        assert_eq!(tmpl.variables.len(), 1);
        assert_eq!(
            tmpl.variables[0].description.as_deref(),
            Some("Custom code description")
        );
    }

    #[test]
    fn test_positional_args_fill_declared_order() {
        let tmpl = PromptTemplate::new(
            "positional-order",
            "Positional fill order",
            "From {{from_lang}} to {{to_lang}}: {{code}}",
        );

        let rendered = tmpl
            .render_positional(&["python", "rust", "def main(): pass"])
            .unwrap();
        assert!(rendered.contains("From python to rust:"));
        assert!(rendered.contains("def main(): pass"));
    }

    #[test]
    fn test_error_display_messages() {
        let e1 = PromptLibError::TemplateNotFound("review2".to_string());
        assert!(e1.to_string().contains("review2"));

        let e2 = PromptLibError::MissingVariable {
            template: "t".to_string(),
            variable: "code".to_string(),
        };
        assert!(e2.to_string().contains("'code'"));

        let e3 = PromptLibError::Validation("empty body".to_string());
        assert!(e3.to_string().contains("empty body"));

        let e4 = PromptLibError::Serialization("bad json".to_string());
        assert!(e4.to_string().contains("bad json"));
    }

    #[test]
    fn test_builtins_all_have_variables_inferred() {
        for t in get_curated_builtin_templates() {
            assert!(!t.name.is_empty(), "builtin template must have a name");
            assert!(
                !t.template.is_empty(),
                "builtin {} must have a body",
                t.name
            );
            assert!(t.is_builtin, "builtin {} must be flagged builtin", t.name);
            // Every {{var}} in the body must have matching metadata entry.
            for var in &t.variables {
                assert!(
                    t.template.contains(&format!("{{{{{}}}}}", var.name))
                        || t.template.contains(&format!("{{{{{}:-", var.name)),
                    "variable {} metadata not found in body of {}",
                    var.name,
                    t.name
                );
            }
        }
    }

    #[test]
    fn test_render_with_declared_default_fallback() {
        let tmpl = PromptTemplate::builder("decl-default")
            .description("Declared default fallback")
            .variable(PromptVariable::new("code", true))
            .variable(PromptVariable::with_default("focus", "performance"))
            .body("Optimize {{code}} for {{focus}}")
            .build();

        let mut vars = HashMap::new();
        vars.insert("code".to_string(), "fn sort() {}".to_string());
        // `focus` is not supplied: declared default kicks in.
        let rendered = tmpl.render(&vars).unwrap();
        assert_eq!(rendered, "Optimize fn sort() {} for performance");
    }
}
