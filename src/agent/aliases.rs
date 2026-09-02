//! User-defined slash command alias subsystem for Fusion.
//!
//! Provides capabilities to:
//! - Define short slash command aliases mapping e.g. `/gp` → `/git push`.
//! - Persist aliases to `~/.fusion/aliases.json`.
//! - Resolve aliases at REPL command-dispatch time before parsing.
//! - Manage aliases via the `/alias` slash command (list, add, remove).

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::config::Config;

// ============================================================================
// Errors
// ============================================================================

/// Errors that can occur during alias operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AliasError {
    /// The alias name is empty or whitespace-only.
    EmptyName,
    /// The alias name does not start with `/`.
    MissingSlash(String),
    /// The alias target is empty.
    EmptyTarget,
    /// The alias target does not start with `/`.
    TargetMissingSlash(String),
    /// The alias name collides with a built-in command.
    BuiltinConflict(String),
    /// File system or I/O error.
    Io(String),
    /// JSON serialization/deserialization error.
    Serialization(String),
}

impl fmt::Display for AliasError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyName => write!(f, "Alias name cannot be empty"),
            Self::MissingSlash(n) => write!(f, "Alias name must start with '/': '{}'", n),
            Self::EmptyTarget => write!(f, "Alias target cannot be empty"),
            Self::TargetMissingSlash(t) => {
                write!(f, "Alias target must start with '/': '{}'", t)
            }
            Self::BuiltinConflict(n) => {
                write!(f, "Cannot shadow built-in command '{}'", n)
            }
            Self::Io(msg) => write!(f, "Alias I/O error: {}", msg),
            Self::Serialization(msg) => write!(f, "Alias serialization error: {}", msg),
        }
    }
}

impl std::error::Error for AliasError {}

impl From<std::io::Error> for AliasError {
    fn from(e: std::io::Error) -> Self {
        AliasError::Io(e.to_string())
    }
}

impl From<serde_json::Error> for AliasError {
    fn from(e: serde_json::Error) -> Self {
        AliasError::Serialization(e.to_string())
    }
}

// ============================================================================
// Data Model
// ============================================================================

/// A single user-defined slash command alias.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Alias {
    /// The short trigger, e.g. `/gp`. Always starts with `/`.
    pub name: String,
    /// The expansion, e.g. `/git push`. Always starts with `/`.
    pub target: String,
    /// Optional human-readable description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl Alias {
    /// Create a new alias without a description.
    pub fn new(name: impl Into<String>, target: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            target: target.into(),
            description: None,
        }
    }

    /// Create a new alias with a description.
    pub fn with_description(
        name: impl Into<String>,
        target: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            target: target.into(),
            description: Some(description.into()),
        }
    }
}

// ============================================================================
// Built-in Command Names (protected from aliasing)
// ============================================================================

/// Names of built-in slash commands that cannot be shadowed.
const BUILTINS: &[&str] = &[
    "/help",
    "/?",
    "/h",
    "/palette",
    "/commands",
    "/pal",
    "/model",
    "/m",
    "/provider",
    "/p",
    "/advisors",
    "/advisor",
    "/adv",
    "/session",
    "/s",
    "/bookmark",
    "/bm",
    "/mark",
    "/fork",
    "/branch",
    "/rewind",
    "/undo",
    "/rw",
    "/compact",
    "/compress",
    "/stats",
    "/usage",
    "/cost",
    "/benchmark",
    "/bench",
    "/latency",
    "/speed",
    "/export",
    "/exp",
    "/trace",
    "/tr",
    "/recover",
    "/recovery",
    "/rec",
    "/clear",
    "/cls",
    "/c",
    "/quit",
    "/exit",
    "/q",
    "/status",
    "/st",
    "/config",
    "/cfg",
    "/preset",
    "/presets",
    "/pre",
    "/tools",
    "/t",
    "/file",
    "/f",
    "/find",
    "/skills",
    "/skill",
    "/sk",
    "/snippet",
    "/snip",
    "/sn",
    "/tag",
    "/tags",
    "/prompt",
    "/prompts",
    "/tmpl",
    "/template",
    "/alias",
    "/aliases",
    "/al",
];

fn is_builtin(name: &str) -> bool {
    let lower = name.to_lowercase();
    BUILTINS.iter().any(|b| *b == lower.as_str())
}

// ============================================================================
// Registry
// ============================================================================

/// Persistent registry of user-defined slash command aliases.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AliasRegistry {
    /// Map from lowercase alias name (e.g. `/gp`) to `Alias`.
    #[serde(default)]
    aliases: HashMap<String, Alias>,
}

impl AliasRegistry {
    /// Returns the path to the aliases file: `~/.fusion/aliases.json`.
    pub fn aliases_path() -> PathBuf {
        Config::config_dir().join("aliases.json")
    }

    /// Load the alias registry from `~/.fusion/aliases.json`.
    ///
    /// Returns an empty registry (without error) if the file does not exist yet.
    pub fn load() -> Result<Self, AliasError> {
        let path = Self::aliases_path();
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = fs::read_to_string(&path)?;
        let registry: Self = serde_json::from_str(&content)?;
        Ok(registry)
    }

    /// Persist the registry to `~/.fusion/aliases.json`, creating parent
    /// directories as needed.
    pub fn save(&self) -> Result<(), AliasError> {
        let path = Self::aliases_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        fs::write(&path, json)?;
        Ok(())
    }

    // -------------------------------------------------------------------------
    // CRUD
    // -------------------------------------------------------------------------

    /// Add or overwrite an alias.
    ///
    /// Validates:
    /// - `name` must start with `/` and be non-empty.
    /// - `target` must start with `/` and be non-empty.
    /// - `name` must not shadow a built-in command.
    pub fn add(
        &mut self,
        name: impl Into<String>,
        target: impl Into<String>,
        description: Option<String>,
    ) -> Result<(), AliasError> {
        let name = name.into();
        let target = target.into();

        let name = normalize_alias_name(&name)?;
        validate_target(&target)?;

        if is_builtin(&name) {
            return Err(AliasError::BuiltinConflict(name));
        }

        let alias = Alias {
            name: name.clone(),
            target,
            description,
        };
        self.aliases.insert(name, alias);
        Ok(())
    }

    /// Remove an alias by name.
    ///
    /// Returns `true` if the alias existed and was removed, `false` otherwise.
    pub fn remove(&mut self, name: &str) -> bool {
        let key = name.trim().to_lowercase();
        self.aliases.remove(&key).is_some()
    }

    /// Resolve an alias: given a raw input string (e.g. `/gp origin main`),
    /// expand the leading command token if it matches an alias.
    ///
    /// Returns:
    /// - `Some(expanded)` when the first token matches a known alias, with any
    ///   trailing arguments appended to the alias target.
    /// - `None` when no alias matches (caller should proceed normally).
    pub fn resolve<'a>(&self, input: &'a str) -> Option<String> {
        let trimmed = input.trim();
        if !trimmed.starts_with('/') {
            return None;
        }

        // Extract command token (up to first whitespace).
        let (cmd_token, rest) = match trimmed.find(char::is_whitespace) {
            Some(pos) => (&trimmed[..pos], trimmed[pos..].trim()),
            None => (trimmed, ""),
        };

        let key = cmd_token.to_lowercase();
        let alias = self.aliases.get(&key)?;

        if rest.is_empty() {
            Some(alias.target.clone())
        } else {
            Some(format!("{} {}", alias.target, rest))
        }
    }

    /// Return all aliases sorted by name.
    pub fn all(&self) -> Vec<&Alias> {
        let mut aliases: Vec<&Alias> = self.aliases.values().collect();
        aliases.sort_by(|a, b| a.name.cmp(&b.name));
        aliases
    }

    /// Return the number of registered aliases.
    pub fn len(&self) -> usize {
        self.aliases.len()
    }

    /// Returns `true` when no aliases are defined.
    pub fn is_empty(&self) -> bool {
        self.aliases.is_empty()
    }

    /// Look up an alias by exact name.
    pub fn get(&self, name: &str) -> Option<&Alias> {
        self.aliases.get(&name.to_lowercase())
    }
}

// ============================================================================
// Validation Helpers
// ============================================================================

fn normalize_alias_name(raw: &str) -> Result<String, AliasError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(AliasError::EmptyName);
    }
    if !trimmed.starts_with('/') {
        return Err(AliasError::MissingSlash(trimmed.to_string()));
    }
    Ok(trimmed.to_lowercase())
}

fn validate_target(raw: &str) -> Result<(), AliasError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(AliasError::EmptyTarget);
    }
    if !trimmed.starts_with('/') {
        return Err(AliasError::TargetMissingSlash(trimmed.to_string()));
    }
    Ok(())
}

// ============================================================================
// `/alias` Slash Command Handler
// ============================================================================

/// Handle the `/alias` slash command.
///
/// Subcommands:
/// - `/alias`                       — list all aliases
/// - `/alias list`                  — same
/// - `/alias add <name> <target>`   — define a new alias
/// - `/alias remove <name>`         — delete an alias
/// - `/alias rm <name>`             — same as remove
/// - `/alias show <name>`           — inspect one alias
pub fn handle_alias_command(args: &[String]) -> String {
    let subcmd = args.first().map(|s| s.as_str()).unwrap_or("list");

    match subcmd {
        "list" | "ls" | "" => cmd_list(),
        "add" | "set" => {
            let rest = &args[1..];
            cmd_add(rest)
        }
        "remove" | "rm" | "delete" | "del" => {
            let name = args.get(1).map(|s| s.as_str()).unwrap_or("");
            cmd_remove(name)
        }
        "show" | "info" => {
            let name = args.get(1).map(|s| s.as_str()).unwrap_or("");
            cmd_show(name)
        }
        _ => {
            // Treat the subcommand position as a name if it starts with '/'.
            // e.g. `/alias /gp` is a shorthand for show.
            if subcmd.starts_with('/') {
                cmd_show(subcmd)
            } else {
                format!(
                    "\x1b[1;31mUnknown alias subcommand:\x1b[0m \x1b[1;37m{}\x1b[0m\n\
                     Usage: /alias [list|add <name> <target>|remove <name>|show <name>]",
                    subcmd
                )
            }
        }
    }
}

fn cmd_list() -> String {
    let registry = match AliasRegistry::load() {
        Ok(r) => r,
        Err(e) => return format!("\x1b[1;31mFailed to load aliases:\x1b[0m {}", e),
    };

    if registry.is_empty() {
        return "\x1b[2mNo aliases defined.\x1b[0m\n\
                Use \x1b[1;36m/alias add /name /target [args]\x1b[0m to create one."
            .to_string();
    }

    let mut out = format!(
        "\x1b[1;36m{} alias(es) defined\x1b[0m\n\n",
        registry.len()
    );

    for alias in registry.all() {
        out.push_str(&format!(
            "  \x1b[1;33m{}\x1b[0m  →  \x1b[1;37m{}\x1b[0m",
            alias.name, alias.target
        ));
        if let Some(ref desc) = alias.description {
            out.push_str(&format!("  \x1b[2m# {}\x1b[0m", desc));
        }
        out.push('\n');
    }

    out
}

fn cmd_add(args: &[String]) -> String {
    // Syntax: add <name> <target...>  [-- description]
    // The target may contain spaces (e.g. `/git push`), so we join remaining tokens.
    if args.len() < 2 {
        return "\x1b[1;31mUsage:\x1b[0m /alias add <name> <target>\n\
                \x1b[2mExample: /alias add /gp \"/git push\"\x1b[0m"
            .to_string();
    }

    let name = args[0].trim();

    // Target is everything from args[1] onward, allowing quoted multi-word targets
    // that were already split by the tokenizer, so we rejoin them.
    let target = args[1..].join(" ");
    let target = target.trim();

    let mut registry = match AliasRegistry::load() {
        Ok(r) => r,
        Err(e) => return format!("\x1b[1;31mFailed to load aliases:\x1b[0m {}", e),
    };

    match registry.add(name, target, None) {
        Ok(()) => {}
        Err(e) => return format!("\x1b[1;31mError:\x1b[0m {}", e),
    }

    if let Err(e) = registry.save() {
        return format!("\x1b[1;31mFailed to save aliases:\x1b[0m {}", e);
    }

    format!(
        "\x1b[1;32mAlias added:\x1b[0m \x1b[1;33m{}\x1b[0m  →  \x1b[1;37m{}\x1b[0m",
        name, target
    )
}

fn cmd_remove(name: &str) -> String {
    if name.is_empty() {
        return "\x1b[1;31mUsage:\x1b[0m /alias remove <name>".to_string();
    }

    let mut registry = match AliasRegistry::load() {
        Ok(r) => r,
        Err(e) => return format!("\x1b[1;31mFailed to load aliases:\x1b[0m {}", e),
    };

    if !registry.remove(name) {
        return format!(
            "\x1b[1;33mAlias \x1b[1;37m{}\x1b[1;33m not found.\x1b[0m",
            name
        );
    }

    if let Err(e) = registry.save() {
        return format!("\x1b[1;31mFailed to save aliases:\x1b[0m {}", e);
    }

    format!(
        "\x1b[1;32mAlias removed:\x1b[0m \x1b[1;33m{}\x1b[0m",
        name
    )
}

fn cmd_show(name: &str) -> String {
    if name.is_empty() {
        return "\x1b[1;31mUsage:\x1b[0m /alias show <name>".to_string();
    }

    let registry = match AliasRegistry::load() {
        Ok(r) => r,
        Err(e) => return format!("\x1b[1;31mFailed to load aliases:\x1b[0m {}", e),
    };

    match registry.get(name) {
        None => format!(
            "\x1b[1;33mAlias \x1b[1;37m{}\x1b[1;33m not found.\x1b[0m",
            name
        ),
        Some(alias) => {
            let mut out = format!(
                "\x1b[1;33m{}\x1b[0m  →  \x1b[1;37m{}\x1b[0m",
                alias.name, alias.target
            );
            if let Some(ref desc) = alias.description {
                out.push_str(&format!("\n  \x1b[2m{}\x1b[0m", desc));
            }
            out
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // AliasRegistry unit tests (no disk I/O)
    // -------------------------------------------------------------------------

    fn registry_with(entries: &[(&str, &str)]) -> AliasRegistry {
        let mut reg = AliasRegistry::default();
        for (name, target) in entries {
            reg.add(*name, *target, None).expect("add should succeed");
        }
        reg
    }

    #[test]
    fn add_and_resolve_basic() {
        let mut reg = AliasRegistry::default();
        reg.add("/gp", "/git push", None).unwrap();
        assert_eq!(reg.resolve("/gp"), Some("/git push".to_string()));
    }

    #[test]
    fn resolve_passes_trailing_args() {
        let reg = registry_with(&[("/gp", "/git push")]);
        assert_eq!(
            reg.resolve("/gp origin main"),
            Some("/git push origin main".to_string())
        );
    }

    #[test]
    fn resolve_returns_none_for_unknown() {
        let reg = registry_with(&[("/gp", "/git push")]);
        assert_eq!(reg.resolve("/unknown"), None);
    }

    #[test]
    fn resolve_returns_none_for_non_slash() {
        let reg = registry_with(&[("/gp", "/git push")]);
        assert_eq!(reg.resolve("hello"), None);
    }

    #[test]
    fn resolve_is_case_insensitive() {
        let reg = registry_with(&[("/GP", "/git push")]);
        assert_eq!(reg.resolve("/gp"), Some("/git push".to_string()));
        assert_eq!(reg.resolve("/GP"), Some("/git push".to_string()));
    }

    #[test]
    fn resolve_exact_match_no_trailing_space() {
        let reg = registry_with(&[("/hi", "/help")]);
        assert_eq!(reg.resolve("/hi"), Some("/help".to_string()));
    }

    #[test]
    fn add_normalizes_name_to_lowercase() {
        let mut reg = AliasRegistry::default();
        reg.add("/GP", "/git push", None).unwrap();
        assert!(reg.get("/gp").is_some());
        assert!(reg.get("/GP").is_some()); // get is also case-insensitive
    }

    #[test]
    fn add_rejects_missing_slash_in_name() {
        let mut reg = AliasRegistry::default();
        let err = reg.add("gp", "/git push", None).unwrap_err();
        assert!(matches!(err, AliasError::MissingSlash(_)));
    }

    #[test]
    fn add_rejects_missing_slash_in_target() {
        let mut reg = AliasRegistry::default();
        let err = reg.add("/gp", "git push", None).unwrap_err();
        assert!(matches!(err, AliasError::TargetMissingSlash(_)));
    }

    #[test]
    fn add_rejects_empty_name() {
        let mut reg = AliasRegistry::default();
        let err = reg.add("", "/git push", None).unwrap_err();
        assert!(matches!(err, AliasError::EmptyName));
    }

    #[test]
    fn add_rejects_empty_target() {
        let mut reg = AliasRegistry::default();
        let err = reg.add("/gp", "", None).unwrap_err();
        assert!(matches!(err, AliasError::EmptyTarget));
    }

    #[test]
    fn add_rejects_builtin_conflict() {
        let mut reg = AliasRegistry::default();
        let err = reg.add("/help", "/something", None).unwrap_err();
        assert!(matches!(err, AliasError::BuiltinConflict(_)));
    }

    #[test]
    fn add_rejects_alias_name_conflict() {
        // `/alias` itself is a builtin
        let mut reg = AliasRegistry::default();
        let err = reg.add("/alias", "/something", None).unwrap_err();
        assert!(matches!(err, AliasError::BuiltinConflict(_)));
    }

    #[test]
    fn remove_existing_alias() {
        let mut reg = registry_with(&[("/gp", "/git push")]);
        assert!(reg.remove("/gp"));
        assert_eq!(reg.resolve("/gp"), None);
    }

    #[test]
    fn remove_nonexistent_returns_false() {
        let mut reg = AliasRegistry::default();
        assert!(!reg.remove("/gp"));
    }

    #[test]
    fn all_returns_sorted_by_name() {
        let reg = registry_with(&[("/z", "/zap"), ("/a", "/ack"), ("/m", "/model")]);
        let names: Vec<&str> = reg.all().iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, &["/a", "/m", "/z"]);
    }

    #[test]
    fn len_and_is_empty() {
        let mut reg = AliasRegistry::default();
        assert!(reg.is_empty());
        reg.add("/x", "/status", None).unwrap();
        assert_eq!(reg.len(), 1);
        assert!(!reg.is_empty());
    }

    #[test]
    fn overwrite_existing_alias() {
        let mut reg = registry_with(&[("/gp", "/git push")]);
        reg.add("/gp", "/git push --force", None).unwrap();
        assert_eq!(reg.resolve("/gp"), Some("/git push --force".to_string()));
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn alias_with_description() {
        let mut reg = AliasRegistry::default();
        reg.add("/gp", "/git push", Some("quick push".to_string()))
            .unwrap();
        let alias = reg.get("/gp").unwrap();
        assert_eq!(alias.description.as_deref(), Some("quick push"));
    }

    // -------------------------------------------------------------------------
    // Serialization round-trip
    // -------------------------------------------------------------------------

    #[test]
    fn serde_round_trip() {
        let mut reg = AliasRegistry::default();
        reg.add("/gp", "/git push", None).unwrap();
        reg.add("/gl", "/git log --oneline", Some("pretty log".to_string()))
            .unwrap();

        let json = serde_json::to_string_pretty(&reg).unwrap();
        let loaded: AliasRegistry = serde_json::from_str(&json).unwrap();

        assert_eq!(loaded.len(), 2);
        assert_eq!(
            loaded.resolve("/gp"),
            Some("/git push".to_string())
        );
        assert_eq!(
            loaded.resolve("/gl"),
            Some("/git log --oneline".to_string())
        );
        assert_eq!(
            loaded.get("/gl").unwrap().description.as_deref(),
            Some("pretty log")
        );
    }

    // -------------------------------------------------------------------------
    // handle_alias_command (string output, no disk I/O needed for basic cases)
    // -------------------------------------------------------------------------

    #[test]
    fn alias_error_display_missing_slash() {
        let e = AliasError::MissingSlash("gp".to_string());
        assert!(e.to_string().contains("must start with '/'"));
    }

    #[test]
    fn alias_error_display_builtin_conflict() {
        let e = AliasError::BuiltinConflict("/help".to_string());
        assert!(e.to_string().contains("built-in"));
    }

    #[test]
    fn is_builtin_recognises_all_variants() {
        assert!(is_builtin("/help"));
        assert!(is_builtin("/HELP"));
        assert!(is_builtin("/q"));
        assert!(is_builtin("/alias"));
        assert!(!is_builtin("/gp"));
        assert!(!is_builtin("/mygit"));
    }

    #[test]
    fn resolve_ignores_whitespace_only_input() {
        let reg = registry_with(&[("/gp", "/git push")]);
        // Leading slash required — pure whitespace is not a slash command
        assert_eq!(reg.resolve("   "), None);
    }

    #[test]
    fn resolve_multi_word_target_expansion() {
        let reg = registry_with(&[("/deploy", "/session new production-deploy")]);
        assert_eq!(
            reg.resolve("/deploy"),
            Some("/session new production-deploy".to_string())
        );
    }

    #[test]
    fn resolve_trailing_args_appended_after_multi_word_target() {
        let reg = registry_with(&[("/deploy", "/session new")]);
        assert_eq!(
            reg.resolve("/deploy my-branch"),
            Some("/session new my-branch".to_string())
        );
    }
}
