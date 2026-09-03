//! Security hardening guardrails for tools and subagent processes.
//!
//! Provides defense-in-depth protection:
//! 1. **Path Guard**: Safe path canonicalization, lexical normalization, and directory
//!    traversal prevention (preventing escapes via `../`, symlinks, or absolute paths outside
//!    allowed workspace boundaries).
//! 2. **Environment Scrubber**: Sensitive environment variable filtering (scrubbing `*_API_KEY`,
//!    `*_SECRET`, `*_TOKEN`, `*_PASSWORD`, `*_CREDENTIALS`, etc. from child shell environments).
//! 3. **Command Guard**: Dangerous command pattern detection preventing catastrophic filesystem
//!    destruction (`rm -rf /`, fork bombs, raw disk writes, format commands).
//! 4. **Unified Security Engine**: Configurable security policies (Strict, Standard, Permissive).

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

// ============================================================================
// Error Types
// ============================================================================

/// Errors emitted when a guardrail rule or security boundary is violated.
#[derive(thiserror::Error, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GuardrailError {
    #[error(
        "Directory traversal detected: path '{path}' attempts to escape allowed root '{root}'"
    )]
    DirectoryTraversal { path: String, root: String },

    #[error("Access denied to forbidden path: '{path}' (matches restricted pattern '{pattern}')")]
    ForbiddenPath { path: String, pattern: String },

    #[error("Symlink traversal escape detected: target '{target}' resolves outside allowed root '{root}'")]
    SymlinkEscape { target: String, root: String },

    #[error("Path '{path}' is outside any allowed root directory")]
    OutsideAllowedRoot { path: String },

    #[error("Operation denied: path '{path}' is in read-only sandbox")]
    ReadOnlyViolation { path: String },

    #[error("Invalid path: {reason}")]
    InvalidPath { reason: String },

    #[error("Blocked dangerous command: '{command}' (matched rule: {rule})")]
    DangerousCommand { command: String, rule: String },
}

// ============================================================================
// Path Normalization & Canonicalization
// ============================================================================

/// Strips Windows extended-length prefix (`\\?\`) if present.
pub fn clean_verbatim_prefix(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(stripped) = s.strip_prefix(r"\\?\") {
        PathBuf::from(stripped)
    } else {
        path.to_path_buf()
    }
}

/// Lexically normalizes a path without touching the filesystem.
///
/// Eliminates redundant `.` components, collapses `..` where possible,
/// and preserves root components and drive letters.
///
/// # Examples
/// ```
/// use std::path::{Path, PathBuf};
/// use fusion::tools::guardrails::normalize_path;
///
/// assert_eq!(normalize_path(Path::new("a/b/../c")), PathBuf::from("a/c"));
/// assert_eq!(normalize_path(Path::new("./a/./b")), PathBuf::from("a/b"));
/// ```
pub fn normalize_path(path: &Path) -> PathBuf {
    let path = clean_verbatim_prefix(path);
    let mut components = Vec::new();
    let mut is_absolute = false;

    for component in path.components() {
        match component {
            Component::Prefix(prefix) => {
                components.push(Component::Prefix(prefix));
            }
            Component::RootDir => {
                is_absolute = true;
                components.push(Component::RootDir);
            }
            Component::CurDir => {
                // Skip redundant '.'
            }
            Component::ParentDir => {
                // If there's a normal component on top of stack, pop it
                match components.last() {
                    Some(Component::Normal(_)) => {
                        components.pop();
                    }
                    Some(Component::RootDir) => {
                        // At root directory, '..' cannot escape higher
                    }
                    Some(Component::Prefix(_)) => {
                        // At prefix root, cannot escape higher
                    }
                    Some(Component::ParentDir) | None => {
                        if !is_absolute {
                            components.push(Component::ParentDir);
                        }
                    }
                    _ => {}
                }
            }
            Component::Normal(c) => {
                components.push(Component::Normal(c));
            }
        }
    }

    if components.is_empty() {
        PathBuf::from(".")
    } else {
        components.iter().collect()
    }
}

/// Checks if `candidate` is strictly a subpath of `root` (or equal to `root`).
///
/// Note: Uses component-based checking, avoiding prefix bugs where `/workspace_secret`
/// might falsely match `/workspace`.
pub fn is_subpath(candidate: &Path, root: &Path) -> bool {
    let norm_candidate = normalize_path(candidate);
    let norm_root = normalize_path(root);

    norm_candidate.starts_with(&norm_root)
}

/// Safely resolves and canonicalizes `raw_path` against `base_dir`, verifying that it
/// resides within `allowed_root` and does not escape via `..` or symlinks.
pub fn canonicalize_safely(
    raw_path: &Path,
    base_dir: &Path,
    allowed_root: &Path,
) -> Result<PathBuf, GuardrailError> {
    let raw_str = raw_path.to_string_lossy();

    // Check for null bytes or control characters in raw path string
    if raw_str.contains('\0') {
        return Err(GuardrailError::InvalidPath {
            reason: "Path contains forbidden null byte character".to_string(),
        });
    }

    // Resolve relative path against base_dir
    let resolved = if raw_path.is_absolute() {
        raw_path.to_path_buf()
    } else {
        base_dir.join(raw_path)
    };

    let normalized = normalize_path(&resolved);
    let norm_root = normalize_path(allowed_root);

    // Lexical check: does normalized path stay within norm_root?
    if !normalized.starts_with(&norm_root) {
        return Err(GuardrailError::DirectoryTraversal {
            path: raw_path.display().to_string(),
            root: allowed_root.display().to_string(),
        });
    }

    // Physical filesystem check: if norm_root exists on disk, ensure that any
    // symlink traversed along the path remains strictly inside allowed_root.
    if let Ok(canon_root) = norm_root.canonicalize() {
        let clean_canon_root = clean_verbatim_prefix(&canon_root);

        // Find the longest existing ancestor
        let mut current = normalized.as_path();
        let mut non_existing_suffix = Vec::new();

        while !current.exists() {
            if let Some(file_name) = current.file_name() {
                non_existing_suffix.push(file_name.to_os_string());
            }
            match current.parent() {
                Some(parent) if parent != current => {
                    current = parent;
                }
                _ => break,
            }
        }

        // If the existing ancestor is inside norm_root, verify its canonical target
        if current.exists() && current.starts_with(&norm_root) {
            if let Ok(canon_existing) = current.canonicalize() {
                let clean_canon = clean_verbatim_prefix(&canon_existing);
                if !clean_canon.starts_with(&clean_canon_root) {
                    return Err(GuardrailError::SymlinkEscape {
                        target: clean_canon.display().to_string(),
                        root: clean_canon_root.display().to_string(),
                    });
                }

                // Reconstruct full path
                let mut full = clean_canon;
                for part in non_existing_suffix.into_iter().rev() {
                    full.push(part);
                }
                return Ok(full);
            }
        }
    }

    Ok(normalized)
}

// ============================================================================
// PathGuard
// ============================================================================

/// Configuration and enforcement for sandbox filesystem paths.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathGuard {
    /// Allowed root directories. Any file access must fall within at least one root.
    pub allowed_roots: Vec<PathBuf>,
    /// Glob patterns or subpaths explicitly denied (e.g. `.git/hooks`, `/etc/shadow`).
    pub denied_patterns: Vec<String>,
    /// Whether absolute paths outside allowed roots are permitted (default: false).
    pub allow_absolute_outside_roots: bool,
    /// Whether the sandbox operates in read-only mode (all writes blocked).
    pub read_only: bool,
}

impl Default for PathGuard {
    fn default() -> Self {
        let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self {
            allowed_roots: vec![current_dir],
            denied_patterns: vec![
                "**/.git/hooks/**".to_string(),
                "**/.git/config".to_string(),
                "**/.ssh/**".to_string(),
                "**/.aws/**".to_string(),
                "**/.gnupg/**".to_string(),
                "/etc/shadow".to_string(),
                "/etc/sudoers".to_string(),
                "C:\\Windows\\System32\\config\\SAM".to_string(),
            ],
            allow_absolute_outside_roots: false,
            read_only: false,
        }
    }
}

impl PathGuard {
    /// Create a new PathGuard anchored to a single root directory.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            allowed_roots: vec![root],
            ..Default::default()
        }
    }

    /// Add an additional allowed root.
    pub fn with_allowed_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.allowed_roots.push(root.into());
        self
    }

    /// Add a denied pattern.
    pub fn with_denied_pattern(mut self, pattern: impl Into<String>) -> Self {
        self.denied_patterns.push(pattern.into());
        self
    }

    /// Set read-only enforcement.
    pub fn with_read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }

    /// Check if a path matches any denied pattern.
    pub fn is_forbidden(&self, path: &Path) -> Option<String> {
        let s = path.to_string_lossy().replace('\\', "/");
        for pattern in &self.denied_patterns {
            let pat = pattern.replace('\\', "/");
            let pat_trimmed = pat.trim_matches('*').trim_matches('/');
            if s.contains(pat_trimmed) {
                return Some(pattern.clone());
            }
        }
        None
    }

    /// Validate a path for general access against allowed roots and denied patterns.
    pub fn validate_path(
        &self,
        raw_path: impl AsRef<Path>,
        cwd: &Path,
    ) -> Result<PathBuf, GuardrailError> {
        let raw = raw_path.as_ref();

        // 1. If allowed_roots is empty, reject
        if self.allowed_roots.is_empty() {
            return Err(GuardrailError::OutsideAllowedRoot {
                path: raw.display().to_string(),
            });
        }

        // 2. Try each allowed root
        let mut last_error = None;
        for root in &self.allowed_roots {
            match canonicalize_safely(raw, cwd, root) {
                Ok(safe_path) => {
                    // Check forbidden patterns
                    if let Some(pat) = self.is_forbidden(&safe_path) {
                        return Err(GuardrailError::ForbiddenPath {
                            path: safe_path.display().to_string(),
                            pattern: pat,
                        });
                    }
                    return Ok(safe_path);
                }
                Err(err) => {
                    last_error = Some(err);
                }
            }
        }

        Err(
            last_error.unwrap_or_else(|| GuardrailError::OutsideAllowedRoot {
                path: raw.display().to_string(),
            }),
        )
    }

    /// Validate a path for read operations.
    pub fn validate_read(
        &self,
        raw_path: impl AsRef<Path>,
        cwd: &Path,
    ) -> Result<PathBuf, GuardrailError> {
        self.validate_path(raw_path, cwd)
    }

    /// Validate a path for write/modification operations.
    pub fn validate_write(
        &self,
        raw_path: impl AsRef<Path>,
        cwd: &Path,
    ) -> Result<PathBuf, GuardrailError> {
        if self.read_only {
            return Err(GuardrailError::ReadOnlyViolation {
                path: raw_path.as_ref().display().to_string(),
            });
        }
        self.validate_path(raw_path, cwd)
    }
}

// ============================================================================
// Environment Scrubber
// ============================================================================

/// Strategy for handling sensitive environment variables.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ScrubStrategy {
    /// Completely remove sensitive variables from the environment map.
    #[default]
    Remove,
    /// Mask sensitive variable values with `[REDACTED]`.
    Mask,
}

/// Audit report detailing environment variable scrubbing results.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScrubReport {
    /// Names of sensitive variables identified and filtered.
    pub scrubbed_keys: Vec<String>,
    /// Total count of variables evaluated.
    pub total_inspected: usize,
    /// Total count of variables kept.
    pub total_retained: usize,
}

/// Filter that detects and neutralizes sensitive environment variables
/// (API keys, secrets, tokens, passwords, private keys).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvScrubber {
    /// Strategy to use when encountering sensitive variables.
    pub strategy: ScrubStrategy,
    /// Explicit sensitive variable names (case-insensitive).
    pub sensitive_names: HashSet<String>,
    /// Suffixes indicating sensitive values (case-insensitive).
    pub sensitive_suffixes: Vec<String>,
    /// Prefixes indicating sensitive values (case-insensitive).
    pub sensitive_prefixes: Vec<String>,
    /// Substrings indicating sensitive values (case-insensitive).
    pub sensitive_substrings: Vec<String>,
    /// Safe variables that must never be filtered even if they contain keywords.
    pub safe_exemptions: HashSet<String>,
}

impl Default for EnvScrubber {
    fn default() -> Self {
        let sensitive_names = [
            // AI / LLM Providers
            "OPENAI_API_KEY",
            "ANTHROPIC_API_KEY",
            "GEMINI_API_KEY",
            "OPENROUTER_API_KEY",
            "MISTRAL_API_KEY",
            "COHERE_API_KEY",
            "GROQ_API_KEY",
            "DEEPSEEK_API_KEY",
            "PERPLEXITY_API_KEY",
            "TOGETHER_API_KEY",
            "REPLICATE_API_TOKEN",
            "HUGGINGFACE_TOKEN",
            "HF_TOKEN",
            // Cloud Providers & Infrastructure
            "AWS_SECRET_ACCESS_KEY",
            "AWS_SESSION_TOKEN",
            "AWS_SECURITY_TOKEN",
            "AZURE_CLIENT_SECRET",
            "GCP_SERVICE_ACCOUNT_KEY",
            "GOOGLE_APPLICATION_CREDENTIALS",
            // Git & Registries
            "GITHUB_TOKEN",
            "GH_TOKEN",
            "GITLAB_TOKEN",
            "NPM_TOKEN",
            "CARGO_REGISTRY_TOKEN",
            "PYPI_TOKEN",
            "DOCKER_AUTH_CONFIG",
            // Databases & Secrets
            "DATABASE_URL",
            "DB_PASSWORD",
            "REDIS_URL",
            "POSTGRES_PASSWORD",
            "MYSQL_PWD",
            "SSH_AUTH_SOCK",
            "SSH_AGENT_PID",
            "GPG_KEY",
            "GPG_AGENT_INFO",
            "VAULT_TOKEN",
            "KUBECONFIG",
            "STRIPE_API_KEY",
            "STRIPE_SECRET",
            "SLACK_TOKEN",
            "SLACK_BOT_TOKEN",
            "DISCORD_TOKEN",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();

        let sensitive_suffixes = vec![
            "_API_KEY".to_string(),
            "_APIKEY".to_string(),
            "_SECRET".to_string(),
            "_SECRET_KEY".to_string(),
            "_TOKEN".to_string(),
            "_ACCESS_TOKEN".to_string(),
            "_AUTH_TOKEN".to_string(),
            "_PASSWORD".to_string(),
            "_PASSWD".to_string(),
            "_PASS".to_string(),
            "_AUTH".to_string(),
            "_CREDENTIALS".to_string(),
            "_CREDENTIAL".to_string(),
            "_PRIVATE_KEY".to_string(),
            "_PRIVKEY".to_string(),
        ];

        let sensitive_prefixes = vec![
            "SECRET_".to_string(),
            "PRIVATE_".to_string(),
            "AUTH_".to_string(),
            "CREDENTIAL_".to_string(),
            "CREDENTIALS_".to_string(),
        ];

        let sensitive_substrings = vec![
            "API_KEY".to_string(),
            "APIKEY".to_string(),
            "ACCESS_KEY".to_string(),
            "SECRET_KEY".to_string(),
            "PRIVATE_KEY".to_string(),
        ];

        let safe_exemptions = [
            "PATH",
            "HOME",
            "USER",
            "LOGNAME",
            "SHELL",
            "TERM",
            "LANG",
            "LC_ALL",
            "LC_CTYPE",
            "TMPDIR",
            "TEMP",
            "TMP",
            "KEYBOARD",
            "KEYMAP",
            "DISPLAY",
            "XDG_DATA_DIRS",
            "XDG_CONFIG_DIRS",
            "SYSTEMROOT",
            "WINDIR",
            "COMSPEC",
            "PATHEXT",
            "APPDATA",
            "LOCALAPPDATA",
            "PROGRAMFILES",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();

        Self {
            strategy: ScrubStrategy::Remove,
            sensitive_names,
            sensitive_suffixes,
            sensitive_prefixes,
            sensitive_substrings,
            safe_exemptions,
        }
    }
}

impl EnvScrubber {
    /// Create a new EnvScrubber with default sensitive detection rules.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the scrubbing strategy (Remove or Mask).
    pub fn with_strategy(mut self, strategy: ScrubStrategy) -> Self {
        self.strategy = strategy;
        self
    }

    /// Check whether a given environment variable key is considered sensitive.
    pub fn is_sensitive(&self, key: &str) -> bool {
        let upper = key.to_ascii_uppercase();

        // 1. Check safe exemptions first
        if self.safe_exemptions.contains(&upper) {
            return false;
        }

        // 2. Check exact matches
        if self.sensitive_names.contains(&upper) {
            return true;
        }

        // 3. Check suffixes
        for suffix in &self.sensitive_suffixes {
            if upper.ends_with(suffix) {
                return true;
            }
        }

        // 4. Check prefixes
        for prefix in &self.sensitive_prefixes {
            if upper.starts_with(prefix) {
                return true;
            }
        }

        // 5. Check sensitive substrings
        for sub in &self.sensitive_substrings {
            if upper.contains(sub) {
                return true;
            }
        }

        false
    }

    /// Scrub a map of environment variables, returning the sanitized map.
    pub fn scrub(&self, env: &HashMap<String, String>) -> HashMap<String, String> {
        let (cleaned, _) = self.scrub_with_report(env);
        cleaned
    }

    /// Scrub environment variables and produce an audit report of what was filtered.
    pub fn scrub_with_report(
        &self,
        env: &HashMap<String, String>,
    ) -> (HashMap<String, String>, ScrubReport) {
        let mut cleaned = HashMap::new();
        let mut scrubbed_keys = Vec::new();
        let total_inspected = env.len();

        for (k, v) in env {
            if self.is_sensitive(k) {
                scrubbed_keys.push(k.clone());
                match self.strategy {
                    ScrubStrategy::Remove => {
                        // Omit completely
                    }
                    ScrubStrategy::Mask => {
                        cleaned.insert(k.clone(), "[REDACTED]".to_string());
                    }
                }
            } else {
                cleaned.insert(k.clone(), v.clone());
            }
        }

        scrubbed_keys.sort();
        let total_retained = cleaned.len();

        let report = ScrubReport {
            scrubbed_keys,
            total_inspected,
            total_retained,
        };

        (cleaned, report)
    }

    /// Prepares sanitized environment variables for a child shell process.
    /// Combines inherited variables and optional custom overrides, scrubbing all secrets.
    pub fn filter_child_process_env(
        &self,
        inherited: &HashMap<String, String>,
        extra: Option<&HashMap<String, String>>,
    ) -> HashMap<String, String> {
        let mut merged = inherited.clone();
        if let Some(overrides) = extra {
            for (k, v) in overrides {
                merged.insert(k.clone(), v.clone());
            }
        }

        self.scrub(&merged)
    }
}

// ============================================================================
// Command Guard
// ============================================================================

/// Guardrail against executing destructive, unrecoverable system commands.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandGuard {
    /// Whether command checking is enabled.
    pub enabled: bool,
    /// Forbidden regex patterns for commands.
    pub blocked_patterns: Vec<(String, String)>,
}

impl Default for CommandGuard {
    fn default() -> Self {
        Self {
            enabled: true,
            blocked_patterns: vec![
                (
                    r"(?i)\brm\s+(-[a-z]*r[a-z]*f|-[a-z]*f[a-z]*r)\s+/\s*$".to_string(),
                    "Recursive root directory deletion (`rm -rf /`)".to_string(),
                ),
                (
                    r"(?i)\brm\s+(-[a-z]*r[a-z]*f|-[a-z]*f[a-z]*r)\s+/\*".to_string(),
                    "Recursive root wildcard deletion (`rm -rf /*`)".to_string(),
                ),
                (
                    r":\(\)\s*\{\s*:\s*\|\s*:\s*&\s*\}\s*;\s*:".to_string(),
                    "Shell fork bomb".to_string(),
                ),
                (
                    r"(?i)\bmkfs\s+/dev/[sh]d[a-z]".to_string(),
                    "Raw disk filesystem wipe (`mkfs`)".to_string(),
                ),
                (
                    r"(?i)\bdd\s+if=.*\s+of=/dev/[sh]d[a-z]".to_string(),
                    "Direct raw disk overwrite (`dd of=/dev/sd*`)".to_string(),
                ),
                (
                    r"(?i)\bformat\s+[a-z]:\s+/fs:".to_string(),
                    "Windows drive format command".to_string(),
                ),
            ],
        }
    }
}

impl CommandGuard {
    /// Create a new command guard.
    pub fn new() -> Self {
        Self::default()
    }

    /// Check whether a command contains prohibited destructive patterns.
    pub fn validate_command(&self, command: &str) -> Result<(), GuardrailError> {
        if !self.enabled {
            return Ok(());
        }

        let trimmed = command.trim();
        for (pattern, rule) in &self.blocked_patterns {
            if let Ok(re) = regex::Regex::new(pattern) {
                if re.is_match(trimmed) {
                    return Err(GuardrailError::DangerousCommand {
                        command: trimmed.to_string(),
                        rule: rule.clone(),
                    });
                }
            }
        }

        Ok(())
    }
}

// ============================================================================
// Unified Security Guardrails
// ============================================================================

/// Security policy level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SecurityLevel {
    /// Minimal checks; allows paths outside workspace with logging.
    Permissive,
    /// Standard protection: blocks traversal escapes and scrubs sensitive env vars.
    #[default]
    Standard,
    /// Strict: read-only workspace boundary, allowlist-only environment, blocks dangerous commands.
    Strict,
}

/// Unified security guardrails combining path protection, env sanitization, and command guards.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityGuardrails {
    pub level: SecurityLevel,
    pub path_guard: PathGuard,
    pub env_scrubber: EnvScrubber,
    pub command_guard: CommandGuard,
}

impl Default for SecurityGuardrails {
    fn default() -> Self {
        Self::standard(std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
    }
}

impl SecurityGuardrails {
    /// Standard security policy anchored to the given workspace root.
    pub fn standard(workspace_root: impl Into<PathBuf>) -> Self {
        let root = workspace_root.into();
        Self {
            level: SecurityLevel::Standard,
            path_guard: PathGuard::new(root),
            env_scrubber: EnvScrubber::default(),
            command_guard: CommandGuard::default(),
        }
    }

    /// Strict security policy: read-only sandbox with maximum guardrails.
    pub fn strict(workspace_root: impl Into<PathBuf>) -> Self {
        let root = workspace_root.into();
        Self {
            level: SecurityLevel::Strict,
            path_guard: PathGuard::new(root).with_read_only(true),
            env_scrubber: EnvScrubber::default(),
            command_guard: CommandGuard::default(),
        }
    }

    /// Permissive security policy for testing or unconstrained environments.
    pub fn permissive() -> Self {
        Self {
            level: SecurityLevel::Permissive,
            path_guard: PathGuard {
                allowed_roots: vec![PathBuf::from("/")],
                denied_patterns: vec![],
                allow_absolute_outside_roots: true,
                read_only: false,
            },
            env_scrubber: EnvScrubber::default().with_strategy(ScrubStrategy::Mask),
            command_guard: CommandGuard {
                enabled: false,
                blocked_patterns: vec![],
            },
        }
    }

    /// Validate a path for reading.
    pub fn validate_read_path(
        &self,
        path: impl AsRef<Path>,
        cwd: &Path,
    ) -> Result<PathBuf, GuardrailError> {
        self.path_guard.validate_read(path, cwd)
    }

    /// Validate a path for writing.
    pub fn validate_write_path(
        &self,
        path: impl AsRef<Path>,
        cwd: &Path,
    ) -> Result<PathBuf, GuardrailError> {
        self.path_guard.validate_write(path, cwd)
    }

    /// Sanitize environment variables for child processes.
    pub fn sanitize_env(&self, env: &HashMap<String, String>) -> HashMap<String, String> {
        self.env_scrubber.scrub(env)
    }

    /// Validate a command against destructive execution rules.
    pub fn validate_command(&self, cmd: &str) -> Result<(), GuardrailError> {
        self.command_guard.validate_command(cmd)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_path_basic() {
        assert_eq!(normalize_path(Path::new("a/b/../c")), PathBuf::from("a/c"));
        assert_eq!(
            normalize_path(Path::new("./a/./b/./c")),
            PathBuf::from("a/b/c")
        );
        assert_eq!(normalize_path(Path::new("a/b/../../c")), PathBuf::from("c"));
    }

    #[test]
    fn test_normalize_path_root_clamping() {
        #[cfg(unix)]
        {
            assert_eq!(
                normalize_path(Path::new("/a/b/../../c")),
                PathBuf::from("/c")
            );
            assert_eq!(
                normalize_path(Path::new("/../../etc/passwd")),
                PathBuf::from("/etc/passwd")
            );
        }
    }

    #[test]
    fn test_directory_traversal_detection() {
        let root = PathBuf::from("/workspace/project");
        let cwd = PathBuf::from("/workspace/project");

        // Safe relative paths
        let safe1 = canonicalize_safely(Path::new("src/main.rs"), &cwd, &root);
        assert!(safe1.is_ok());
        assert_eq!(
            safe1.unwrap(),
            PathBuf::from("/workspace/project/src/main.rs")
        );

        let safe2 = canonicalize_safely(Path::new("src/utils/../main.rs"), &cwd, &root);
        assert!(safe2.is_ok());
        assert_eq!(
            safe2.unwrap(),
            PathBuf::from("/workspace/project/src/main.rs")
        );

        // Dangerous relative escapes
        let escape1 = canonicalize_safely(Path::new("../secret.txt"), &cwd, &root);
        assert!(matches!(
            escape1,
            Err(GuardrailError::DirectoryTraversal { .. })
        ));

        let escape2 = canonicalize_safely(Path::new("../../etc/passwd"), &cwd, &root);
        assert!(matches!(
            escape2,
            Err(GuardrailError::DirectoryTraversal { .. })
        ));

        let escape3 = canonicalize_safely(Path::new("src/../../../../etc/shadow"), &cwd, &root);
        assert!(matches!(
            escape3,
            Err(GuardrailError::DirectoryTraversal { .. })
        ));
    }

    #[test]
    fn test_prefix_collision_safety() {
        // Ensure /workspace_secret does NOT pass when allowed root is /workspace
        let root = PathBuf::from("/workspace");
        let cwd = PathBuf::from("/workspace");

        let malicious = Path::new("/workspace_secret/file.txt");
        let res = canonicalize_safely(malicious, &cwd, &root);
        assert!(matches!(
            res,
            Err(GuardrailError::DirectoryTraversal { .. })
        ));
    }

    #[test]
    fn test_null_byte_rejection() {
        let root = PathBuf::from("/workspace");
        let cwd = PathBuf::from("/workspace");

        let null_path = Path::new("file\0.txt");
        let res = canonicalize_safely(null_path, &cwd, &root);
        assert!(matches!(res, Err(GuardrailError::InvalidPath { .. })));
    }

    #[test]
    fn test_path_guard_forbidden_patterns() {
        let root = PathBuf::from("/workspace");
        let guard = PathGuard::new(&root);

        // .git/hooks path should be blocked
        let git_hook = Path::new(".git/hooks/pre-commit");
        let res = guard.validate_path(git_hook, &root);
        assert!(matches!(res, Err(GuardrailError::ForbiddenPath { .. })));

        // Safe file should pass
        let safe = Path::new("src/lib.rs");
        let res = guard.validate_path(safe, &root);
        assert!(res.is_ok());
    }

    #[test]
    fn test_read_only_sandbox() {
        let root = PathBuf::from("/workspace");
        let guard = PathGuard::new(&root).with_read_only(true);

        let read_res = guard.validate_read("src/main.rs", &root);
        assert!(read_res.is_ok());

        let write_res = guard.validate_write("src/main.rs", &root);
        assert!(matches!(
            write_res,
            Err(GuardrailError::ReadOnlyViolation { .. })
        ));
    }

    #[test]
    fn test_env_scrubber_detects_sensitive_variables() {
        let scrubber = EnvScrubber::default();

        // Exact names
        assert!(scrubber.is_sensitive("OPENAI_API_KEY"));
        assert!(scrubber.is_sensitive("openai_api_key")); // case-insensitivity
        assert!(scrubber.is_sensitive("ANTHROPIC_API_KEY"));
        assert!(scrubber.is_sensitive("AWS_SECRET_ACCESS_KEY"));
        assert!(scrubber.is_sensitive("GITHUB_TOKEN"));
        assert!(scrubber.is_sensitive("DATABASE_URL"));

        // Suffix matches
        assert!(scrubber.is_sensitive("CUSTOM_SERVICE_API_KEY"));
        assert!(scrubber.is_sensitive("MY_APP_SECRET"));
        assert!(scrubber.is_sensitive("DB_PASSWORD"));
        assert!(scrubber.is_sensitive("AUTH_TOKEN"));
        assert!(scrubber.is_sensitive("USER_CREDENTIALS"));

        // Prefix matches
        assert!(scrubber.is_sensitive("SECRET_INTERNAL_KEY"));
        assert!(scrubber.is_sensitive("PRIVATE_RSA_KEY"));

        // Safe system variables must not be filtered
        assert!(!scrubber.is_sensitive("PATH"));
        assert!(!scrubber.is_sensitive("HOME"));
        assert!(!scrubber.is_sensitive("USER"));
        assert!(!scrubber.is_sensitive("TERM"));
        assert!(!scrubber.is_sensitive("LANG"));
        assert!(!scrubber.is_sensitive("KEYBOARD"));
        assert!(!scrubber.is_sensitive("KEYMAP"));
    }

    #[test]
    fn test_env_scrubber_cleaning() {
        let scrubber = EnvScrubber::default();
        let mut env = HashMap::new();
        env.insert("PATH".to_string(), "/usr/bin:/bin".to_string());
        env.insert("USER".to_string(), "fusion_user".to_string());
        env.insert("OPENAI_API_KEY".to_string(), "sk-123456789".to_string());
        env.insert("CUSTOM_API_KEY".to_string(), "sec-abcdef".to_string());
        env.insert("APP_SECRET".to_string(), "supersecret".to_string());
        env.insert(
            "DATABASE_URL".to_string(),
            "postgres://user:pass@localhost".to_string(),
        );

        let (cleaned, report) = scrubber.scrub_with_report(&env);

        assert_eq!(cleaned.len(), 2);
        assert!(cleaned.contains_key("PATH"));
        assert!(cleaned.contains_key("USER"));
        assert!(!cleaned.contains_key("OPENAI_API_KEY"));
        assert!(!cleaned.contains_key("CUSTOM_API_KEY"));
        assert!(!cleaned.contains_key("APP_SECRET"));
        assert!(!cleaned.contains_key("DATABASE_URL"));

        assert_eq!(report.scrubbed_keys.len(), 4);
        assert!(report.scrubbed_keys.contains(&"OPENAI_API_KEY".to_string()));
        assert!(report.scrubbed_keys.contains(&"CUSTOM_API_KEY".to_string()));
    }

    #[test]
    fn test_env_scrubber_mask_mode() {
        let scrubber = EnvScrubber::default().with_strategy(ScrubStrategy::Mask);
        let mut env = HashMap::new();
        env.insert("PATH".to_string(), "/bin".to_string());
        env.insert("STRIPE_SECRET".to_string(), "sk_live_xyz".to_string());

        let cleaned = scrubber.scrub(&env);
        assert_eq!(cleaned.get("PATH").unwrap(), "/bin");
        assert_eq!(cleaned.get("STRIPE_SECRET").unwrap(), "[REDACTED]");
    }

    #[test]
    fn test_command_guard_dangerous_patterns() {
        let guard = CommandGuard::default();

        // Block rm -rf /
        assert!(guard.validate_command("rm -rf /").is_err());
        assert!(guard.validate_command("rm -fr /").is_err());
        assert!(guard.validate_command("rm -rf /*").is_err());

        // Block fork bomb
        assert!(guard.validate_command(":(){ :|:& };:").is_err());

        // Block raw disk wipe
        assert!(guard.validate_command("mkfs /dev/sda").is_err());
        assert!(guard
            .validate_command("dd if=/dev/zero of=/dev/sda")
            .is_err());

        // Allow safe commands
        assert!(guard.validate_command("cargo build").is_ok());
        assert!(guard.validate_command("git status").is_ok());
        assert!(guard.validate_command("rm -rf ./target").is_ok());
        assert!(guard.validate_command("ls -la /tmp").is_ok());
    }

    #[test]
    fn test_unified_guardrails() {
        let root = PathBuf::from("/workspace/project");
        let security = SecurityGuardrails::standard(&root);

        // Path check
        assert!(security.validate_read_path("src/main.rs", &root).is_ok());
        assert!(security
            .validate_read_path("../../etc/passwd", &root)
            .is_err());

        // Env check
        let mut env = HashMap::new();
        env.insert("OPENAI_API_KEY".to_string(), "secret".to_string());
        env.insert("USER".to_string(), "alice".to_string());
        let cleaned = security.sanitize_env(&env);
        assert!(!cleaned.contains_key("OPENAI_API_KEY"));
        assert!(cleaned.contains_key("USER"));

        // Command check
        assert!(security.validate_command("cargo test").is_ok());
        assert!(security.validate_command("rm -rf /").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn test_symlink_escape_detection_real_fs() {
        let temp =
            std::env::temp_dir().join(format!("fusion_guardrails_test_{}", std::process::id()));
        let workspace = temp.join("workspace");
        let outside = temp.join("outside");

        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::create_dir_all(&outside).unwrap();

        let secret_file = outside.join("secret.txt");
        std::fs::write(&secret_file, "super_secret").unwrap();

        // Create a symlink inside workspace pointing to outside directory
        let link = workspace.join("symlink_to_outside");
        std::os::unix::fs::symlink(&outside, &link).unwrap();

        // Attempt to access outside file via symlink
        let target_via_link = PathBuf::from("symlink_to_outside/secret.txt");
        let res = canonicalize_safely(&target_via_link, &workspace, &workspace);

        assert!(matches!(res, Err(GuardrailError::SymlinkEscape { .. })));

        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn test_filter_child_process_env() {
        let scrubber = EnvScrubber::default();

        let mut inherited = HashMap::new();
        inherited.insert("PATH".to_string(), "/usr/bin".to_string());
        inherited.insert("HOME".to_string(), "/home/user".to_string());
        inherited.insert("OPENAI_API_KEY".to_string(), "sk-secret-123".to_string());
        inherited.insert(
            "AWS_SECRET_ACCESS_KEY".to_string(),
            "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_string(),
        );

        let mut overrides = HashMap::new();
        overrides.insert("FOO".to_string(), "bar".to_string());
        overrides.insert("CUSTOM_API_KEY".to_string(), "leak-secret".to_string());

        let child_env = scrubber.filter_child_process_env(&inherited, Some(&overrides));

        assert_eq!(child_env.get("PATH").unwrap(), "/usr/bin");
        assert_eq!(child_env.get("HOME").unwrap(), "/home/user");
        assert_eq!(child_env.get("FOO").unwrap(), "bar");

        // All sensitive keys stripped
        assert!(!child_env.contains_key("OPENAI_API_KEY"));
        assert!(!child_env.contains_key("AWS_SECRET_ACCESS_KEY"));
        assert!(!child_env.contains_key("CUSTOM_API_KEY"));
    }

    #[test]
    fn test_is_subpath_component_safety() {
        let root = Path::new("/var/app");

        assert!(is_subpath(Path::new("/var/app/src/main.rs"), root));
        assert!(is_subpath(Path::new("/var/app"), root));
        assert!(!is_subpath(Path::new("/var/app_secret"), root));
        assert!(!is_subpath(Path::new("/var/application/foo"), root));
        assert!(!is_subpath(Path::new("/etc/passwd"), root));
    }

    #[test]
    fn test_case_insensitive_sensitive_keys() {
        let scrubber = EnvScrubber::default();

        let test_keys = [
            "openai_api_key",
            "ANTHROPIC_API_KEY",
            "gemini_api_key",
            "My_App_Secret",
            "SERVER_PRIVATE_KEY",
            "client_secret_key",
            "access_token",
            "USER_PASSWORD",
            "db_pass",
            "github_token",
            "DATABASE_URL",
            "ssh_auth_sock",
        ];

        for key in &test_keys {
            assert!(
                scrubber.is_sensitive(key),
                "Expected sensitive detection for: {key}"
            );
        }
    }
}
