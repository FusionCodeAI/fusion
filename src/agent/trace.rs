use chrono::Utc;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, RwLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

use crate::agent::loop_runner::AgentRunner;
use crate::agent::session::{Session, TokenStats};
use crate::config::Config;
use crate::provider::types::{Message, Role, ToolCall};
// ---------------------------------------------------------------------------
// Redaction & Privacy Engine
// ---------------------------------------------------------------------------

/// Categorization of privacy redactions performed during diagnostic trace generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RedactionCategory {
    /// LLM provider and cloud API keys (e.g. Anthropic, OpenAI, OpenRouter, Google, AWS).
    ApiKey,
    /// Bearer tokens, GitHub tokens, Slack tokens, and session tokens.
    AuthToken,
    /// Passwords, client secrets, and credential values in strings, JSON, or CLI args.
    PasswordOrSecret,
    /// Asymmetric private keys (PEM format RSA, EC, ED25519, OpenSSH).
    PrivateKey,
    /// User filesystem paths containing home directories or local user identities.
    UserPath,
    /// Personal or corporate email addresses.
    Email,
    /// IPv4 and IPv6 network addresses (excluding loopback 127.0.0.1 and ::1).
    IpAddress,
    /// Custom user-specified sensitive strings or patterns.
    Custom,
}

/// Audit report tracking all privacy redactions applied during trace generation.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedactionAudit {
    /// Number of provider and cloud API keys redacted.
    pub api_keys: usize,
    /// Number of auth and bearer tokens redacted.
    pub auth_tokens: usize,
    /// Number of passwords, credentials, and secrets redacted.
    pub passwords_and_secrets: usize,
    /// Number of cryptographic private keys redacted.
    pub private_keys: usize,
    /// Number of home and user paths anonymized.
    pub user_paths: usize,
    /// Number of email addresses redacted.
    pub emails: usize,
    /// Number of IP addresses redacted.
    pub ip_addresses: usize,
    /// Number of custom sensitive terms redacted.
    pub custom: usize,
    /// Total count of all redactions performed.
    pub total_redactions: usize,
}

impl RedactionAudit {
    /// Records an occurrence of a specific redaction category.
    pub fn record(&mut self, category: RedactionCategory, count: usize) {
        if count == 0 {
            return;
        }
        match category {
            RedactionCategory::ApiKey => self.api_keys += count,
            RedactionCategory::AuthToken => self.auth_tokens += count,
            RedactionCategory::PasswordOrSecret => self.passwords_and_secrets += count,
            RedactionCategory::PrivateKey => self.private_keys += count,
            RedactionCategory::UserPath => self.user_paths += count,
            RedactionCategory::Email => self.emails += count,
            RedactionCategory::IpAddress => self.ip_addresses += count,
            RedactionCategory::Custom => self.custom += count,
        }
        self.total_redactions += count;
    }

    /// Returns true if no sensitive items were detected or redacted.
    pub fn is_clean(&self) -> bool {
        self.total_redactions == 0
    }

    /// Formats a human-readable one-line summary of redactions.
    pub fn summary(&self) -> String {
        if self.is_clean() {
            "0 redactions applied (clean)".to_string()
        } else {
            format!(
                "{} redaction{} applied (keys: {}, tokens: {}, secrets: {}, paths: {}, emails: {}, ips: {})",
                self.total_redactions,
                if self.total_redactions == 1 { "" } else { "s" },
                self.api_keys,
                self.auth_tokens,
                self.passwords_and_secrets,
                self.user_paths,
                self.emails,
                self.ip_addresses,
            )
        }
    }
}

/// Core redaction engine responsible for sanitizing diagnostic traces.
#[derive(Debug, Clone)]
pub struct TraceRedactor {
    home_dir: Option<String>,
    home_dir_raw: Option<PathBuf>,
    exact_secrets: Vec<String>,
    custom_patterns: Vec<Regex>,
}

impl Default for TraceRedactor {
    fn default() -> Self {
        Self::new()
    }
}

impl TraceRedactor {
    /// Creates a new redactor initialized with system home directory detection.
    pub fn new() -> Self {
        let home_dir_raw = dirs::home_dir();
        let home_dir = home_dir_raw
            .as_ref()
            .map(|p| p.to_string_lossy().to_string());

        Self {
            home_dir,
            home_dir_raw,
            exact_secrets: Vec::new(),
            custom_patterns: Vec::new(),
        }
    }
    /// Returns the detected user home directory string if available.
    pub fn home_dir(&self) -> Option<&str> {
        self.home_dir.as_deref()
    }

    /// Returns the detected user home directory as a Path reference if available.
    pub fn home_dir_path(&self) -> Option<&Path> {
        self.home_dir_raw.as_deref()
    }

    /// Registers known secret strings (such as active API keys from configuration) to be strictly scrubbed.
    pub fn with_known_secrets(mut self, secrets: &[&str]) -> Self {
        for s in secrets {
            let trimmed = s.trim();
            if trimmed.len() >= 6
                && !self
                    .exact_secrets
                    .iter()
                    .any(|existing| existing == trimmed)
            {
                self.exact_secrets.push(trimmed.to_string());
            }
        }
        self
    }

    /// Adds custom regex patterns to be redacted as `[REDACTED_CUSTOM]`.
    pub fn with_custom_pattern(mut self, pattern: &str) -> Result<Self, regex::Error> {
        let regex = Regex::new(pattern)?;
        self.custom_patterns.push(regex);
        Ok(self)
    }

    /// Sanitizes the given input text, tracking redactions in the audit accumulator.
    pub fn redact(&self, input: &str, audit: &mut RedactionAudit) -> String {
        if input.is_empty() {
            return String::new();
        }

        let mut text = input.to_string();

        // 1. Exact known secrets scrub (e.g. active API keys in Config)
        for secret in &self.exact_secrets {
            if text.contains(secret) {
                let matches = text.matches(secret).count();
                text = text.replace(secret, "[REDACTED_API_KEY]");
                audit.record(RedactionCategory::ApiKey, matches);
            }
        }

        // 2. Private Keys (PEM format RSA, EC, OpenSSH, PGP)
        // Regex handles multi-line PEM blocks
        let pem_regex = Regex::new(
            r"(?s)-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----.*?-----END [A-Z0-9 ]*PRIVATE KEY-----",
        )
        .expect("valid regex");
        let pem_matches = pem_regex.find_iter(&text).count();
        if pem_matches > 0 {
            text = pem_regex
                .replace_all(&text, "[REDACTED_PRIVATE_KEY]")
                .to_string();
            audit.record(RedactionCategory::PrivateKey, pem_matches);
        }

        // 3. Provider & Cloud API Keys
        // Anthropic: sk-ant-api03-... or sk-ant-...
        let anthropic_regex =
            Regex::new(r"sk-ant-(?:api03-)?[A-Za-z0-9_-]{20,}").expect("valid regex");
        let ant_count = anthropic_regex.find_iter(&text).count();
        if ant_count > 0 {
            text = anthropic_regex
                .replace_all(&text, "[REDACTED_API_KEY]")
                .to_string();
            audit.record(RedactionCategory::ApiKey, ant_count);
        }

        // OpenAI: sk-proj-... or sk-...
        let openai_regex = Regex::new(r"sk-(?:proj-)?[A-Za-z0-9_-]{20,}").expect("valid regex");
        let oai_count = openai_regex.find_iter(&text).count();
        if oai_count > 0 {
            text = openai_regex
                .replace_all(&text, "[REDACTED_API_KEY]")
                .to_string();
            audit.record(RedactionCategory::ApiKey, oai_count);
        }

        // OpenRouter: sk-or-v1-...
        let openrouter_regex =
            Regex::new(r"sk-or-(?:v1-)?[A-Za-z0-9_-]{20,}").expect("valid regex");
        let or_count = openrouter_regex.find_iter(&text).count();
        if or_count > 0 {
            text = openrouter_regex
                .replace_all(&text, "[REDACTED_API_KEY]")
                .to_string();
            audit.record(RedactionCategory::ApiKey, or_count);
        }

        // Google Gemini / Cloud: AIzaSy...
        let google_regex = Regex::new(r"AIza[0-9A-Za-z_-]{35}").expect("valid regex");
        let g_count = google_regex.find_iter(&text).count();
        if g_count > 0 {
            text = google_regex
                .replace_all(&text, "[REDACTED_API_KEY]")
                .to_string();
            audit.record(RedactionCategory::ApiKey, g_count);
        }

        // AWS Access Key ID: AKIA...
        let aws_regex = Regex::new(r"\bAKIA[0-9A-Z]{16}\b").expect("valid regex");
        let aws_count = aws_regex.find_iter(&text).count();
        if aws_count > 0 {
            text = aws_regex
                .replace_all(&text, "[REDACTED_AWS_KEY]")
                .to_string();
            audit.record(RedactionCategory::ApiKey, aws_count);
        }

        // HuggingFace: hf_...
        let hf_regex = Regex::new(r"hf_[A-Za-z0-9]{30,}").expect("valid regex");
        let hf_count = hf_regex.find_iter(&text).count();
        if hf_count > 0 {
            text = hf_regex
                .replace_all(&text, "[REDACTED_API_KEY]")
                .to_string();
            audit.record(RedactionCategory::ApiKey, hf_count);
        }

        // 4. Auth & Bearer Tokens
        // GitHub: ghp_..., gho_..., github_pat_...
        let github_regex =
            Regex::new(r"\b(?:gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{22,})\b")
                .expect("valid regex");
        let gh_count = github_regex.find_iter(&text).count();
        if gh_count > 0 {
            text = github_regex
                .replace_all(&text, "[REDACTED_GITHUB_TOKEN]")
                .to_string();
            audit.record(RedactionCategory::AuthToken, gh_count);
        }

        // Slack: xoxb-..., xoxp-..., xoxa-...
        let slack_regex = Regex::new(r"xox[baprs]-[0-9A-Za-z-]{10,}").expect("valid regex");
        let slack_count = slack_regex.find_iter(&text).count();
        if slack_count > 0 {
            text = slack_regex
                .replace_all(&text, "[REDACTED_SLACK_TOKEN]")
                .to_string();
            audit.record(RedactionCategory::AuthToken, slack_count);
        }

        // Bearer tokens in headers or strings: Bearer <token>
        let bearer_regex =
            Regex::new(r"(?i)\bBearer\s+([A-Za-z0-9_\-\.]{16,})\b").expect("valid regex");
        let bearer_count = bearer_regex.find_iter(&text).count();
        if bearer_count > 0 {
            text = bearer_regex
                .replace_all(&text, "Bearer [REDACTED_TOKEN]")
                .to_string();
            audit.record(RedactionCategory::AuthToken, bearer_count);
        }

        // 5. Passwords & Secrets in JSON, URL query parameters, and CLI flags
        // JSON fields: "password": "...", "secret": "...", "client_secret": "..."
        let json_secret_regex = Regex::new(
            r#"(?i)("(?:password|passwd|secret|client_secret|api_key|apikey|access_token|refresh_token|private_key)"\s*:\s*)"([^"]+)""#,
        )
        .expect("valid regex");
        let json_matches = json_secret_regex.find_iter(&text).count();
        if json_matches > 0 {
            text = json_secret_regex
                .replace_all(&text, r#"$1"[REDACTED_SECRET]""#)
                .to_string();
            audit.record(RedactionCategory::PasswordOrSecret, json_matches);
        }

        // CLI flags: --password <val>, --secret <val>, --api-key <val>
        let cli_secret_regex =
            Regex::new(r"(?i)(--(?:password|passwd|secret|api-key|token)\s+)([^\s]+)")
                .expect("valid regex");
        let cli_matches = cli_secret_regex.find_iter(&text).count();
        if cli_matches > 0 {
            text = cli_secret_regex
                .replace_all(&text, "$1[REDACTED_SECRET]")
                .to_string();
            audit.record(RedactionCategory::PasswordOrSecret, cli_matches);
        }

        // Connection string credentials: postgres://user:password@host
        let uri_cred_regex =
            Regex::new(r"(?i)([a-z][a-z0-9+.-]*://[^:\s]+:)([^@\s]+)(@)").expect("valid regex");
        let uri_matches = uri_cred_regex.find_iter(&text).count();
        if uri_matches > 0 {
            text = uri_cred_regex
                .replace_all(&text, "$1[REDACTED_PASSWORD]$3")
                .to_string();
            audit.record(RedactionCategory::PasswordOrSecret, uri_matches);
        }

        // 6. User Home & Identity Paths
        // Current user home directory
        if let Some(home) = &self.home_dir {
            if !home.is_empty() && text.contains(home) {
                let home_count = text.matches(home).count();
                text = text.replace(home, "~");
                audit.record(RedactionCategory::UserPath, home_count);
            }
        }

        // Generic Unix / macOS user paths: /Users/<user>/ or /home/<user>/
        let unix_user_regex =
            Regex::new(r"/(?:Users|home)/([a-zA-Z0-9_.-]+)").expect("valid regex");
        let mut path_count = 0;
        text = unix_user_regex
            .replace_all(&text, |caps: &regex::Captures| {
                let matched_user = &caps[1];
                if matched_user == "[USER]" || matched_user.is_empty() {
                    caps[0].to_string()
                } else {
                    path_count += 1;
                    let prefix = if caps[0].starts_with("/Users") {
                        "/Users"
                    } else {
                        "/home"
                    };
                    format!("{}/[USER]", prefix)
                }
            })
            .to_string();
        if path_count > 0 {
            audit.record(RedactionCategory::UserPath, path_count);
        }

        // Windows user paths: C:\Users\<user>\
        let win_user_regex =
            Regex::new(r"(?i)[a-z]:\\Users\\([a-zA-Z0-9_.-]+)").expect("valid regex");
        let mut win_count = 0;
        text = win_user_regex
            .replace_all(&text, |caps: &regex::Captures| {
                let matched_user = &caps[1];
                if matched_user == "[USER]" || matched_user.is_empty() {
                    caps[0].to_string()
                } else {
                    win_count += 1;
                    r"C:\Users\[USER]".to_string()
                }
            })
            .to_string();
        if win_count > 0 {
            audit.record(RedactionCategory::UserPath, win_count);
        }

        // 7. Email Addresses
        let email_regex =
            Regex::new(r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b").expect("valid regex");
        let email_count = email_regex.find_iter(&text).count();
        if email_count > 0 {
            text = email_regex
                .replace_all(&text, "[REDACTED_EMAIL]")
                .to_string();
            audit.record(RedactionCategory::Email, email_count);
        }

        // 8. IP Addresses (IPv4, preserving 127.0.0.1, 0.0.0.0, and standard subnets if necessary)
        let ip_regex = Regex::new(r"\b\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}\b").expect("valid regex");
        let mut ip_count = 0;
        text = ip_regex
            .replace_all(&text, |caps: &regex::Captures| {
                let ip_str = &caps[0];
                if ip_str == "127.0.0.1" || ip_str == "0.0.0.0" {
                    ip_str.to_string()
                } else {
                    // Check if it's a valid octet range
                    let valid_octets = ip_str
                        .split('.')
                        .filter_map(|s| s.parse::<u8>().ok())
                        .count()
                        == 4;
                    if valid_octets {
                        ip_count += 1;
                        "[REDACTED_IP]".to_string()
                    } else {
                        ip_str.to_string()
                    }
                }
            })
            .to_string();
        if ip_count > 0 {
            audit.record(RedactionCategory::IpAddress, ip_count);
        }

        // 9. Custom user-supplied patterns
        for custom_re in &self.custom_patterns {
            let count = custom_re.find_iter(&text).count();
            if count > 0 {
                text = custom_re
                    .replace_all(&text, "[REDACTED_CUSTOM]")
                    .to_string();
                audit.record(RedactionCategory::Custom, count);
            }
        }

        text
    }
}

// ---------------------------------------------------------------------------
// System Info Collector
// ---------------------------------------------------------------------------

/// System environment information gathered without leaking sensitive data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemInfo {
    /// Operating system name (e.g. "macos", "linux", "windows").
    pub os: String,
    /// Operating system family (e.g. "unix", "windows").
    pub os_family: String,
    /// CPU target architecture (e.g. "aarch64", "x86_64").
    pub arch: String,
    /// Memory pointer width in bits (e.g. 64).
    pub pointer_width: usize,
    /// Number of logical CPU cores available.
    pub cpu_cores: usize,
    /// Fusion package version.
    pub fusion_version: String,
    /// Target platform triple or fallback.
    pub target_platform: String,
    /// Active shell executable name.
    pub shell: Option<String>,
    /// Terminal identifier or emulator name.
    pub terminal: Option<String>,
    /// Safe environment variables (whitelist only).
    pub safe_env_vars: Vec<(String, String)>,
    /// Status of configured provider API keys (presence only, values never stored).
    pub provider_keys_status: Vec<(String, bool)>,
    /// Git repository status if the working directory is inside a repository.
    pub git_info: Option<GitInfo>,
}

/// Lightweight, privacy-safe Git repository status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitInfo {
    /// Active branch or HEAD reference name.
    pub branch: String,
    /// Short commit hash of HEAD.
    pub short_commit: String,
    /// Whether the working tree contains uncommitted modifications.
    pub is_dirty: bool,
}

impl SystemInfo {
    /// Collects system environment information safely.
    pub fn collect(
        config: Option<&Config>,
        redactor: &TraceRedactor,
        audit: &mut RedactionAudit,
    ) -> Self {
        let os = std::env::consts::OS.to_string();
        let os_family = std::env::consts::FAMILY.to_string();
        let arch = std::env::consts::ARCH.to_string();
        let pointer_width = usize::BITS as usize;
        let cpu_cores = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1);
        let fusion_version = env!("CARGO_PKG_VERSION").to_string();
        let target_platform = format!("{}-{}", arch, os);

        // Safe Shell inspection (basename only to avoid user path leak)
        let shell = std::env::var("SHELL")
            .or_else(|_| std::env::var("COMSPEC"))
            .ok()
            .map(|s| {
                Path::new(&s)
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or(s)
            });

        // Terminal inspection
        let terminal = std::env::var("TERM_PROGRAM")
            .or_else(|_| std::env::var("TERM"))
            .ok();

        // Safe environment variables whitelist (never include any credentials or secret tokens)
        let safe_whitelist = [
            "SHELL",
            "TERM",
            "TERM_PROGRAM",
            "LANG",
            "LC_ALL",
            "RUST_LOG",
            "CI",
            "TERMUX_VERSION",
            "FUSION_MODEL",
            "FUSION_PROVIDER",
        ];

        let mut safe_env_vars = Vec::new();
        for key in safe_whitelist {
            if let Ok(val) = std::env::var(key) {
                let sanitized_val = redactor.redact(&val, audit);
                safe_env_vars.push((key.to_string(), sanitized_val));
            }
        }

        // Provider configuration readiness (presence only)
        let mut provider_keys_status = Vec::new();
        if let Some(cfg) = config {
            provider_keys_status.push(("deepseek".to_string(), cfg.deepseek_api_key.is_some()));
            provider_keys_status.push(("anthropic".to_string(), cfg.anthropic_api_key.is_some()));
            provider_keys_status.push(("openai".to_string(), cfg.openai_api_key.is_some()));
            provider_keys_status.push(("xai".to_string(), cfg.xai_api_key.is_some()));
            provider_keys_status.push(("openrouter".to_string(), cfg.openrouter_api_key.is_some()));
            provider_keys_status.push(("ollama".to_string(), true)); // Local provider
        }

        // Git repository status
        let git_info = Self::collect_git_info();

        Self {
            os,
            os_family,
            arch,
            pointer_width,
            cpu_cores,
            fusion_version,
            target_platform,
            shell,
            terminal,
            safe_env_vars,
            provider_keys_status,
            git_info,
        }
    }

    /// Attempts to read Git status in a lightweight, non-blocking way.
    fn collect_git_info() -> Option<GitInfo> {
        // Run git rev-parse to check if we are in a work tree
        let branch_output = std::process::Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .output()
            .ok()?;

        if !branch_output.status.success() {
            return None;
        }

        let branch = String::from_utf8_lossy(&branch_output.stdout)
            .trim()
            .to_string();

        let commit_output = std::process::Command::new("git")
            .args(["rev-parse", "--short", "HEAD"])
            .output()
            .ok()?;
        let short_commit = String::from_utf8_lossy(&commit_output.stdout)
            .trim()
            .to_string();

        let status_output = std::process::Command::new("git")
            .args(["status", "--porcelain"])
            .output()
            .ok()?;
        let is_dirty = !status_output.stdout.is_empty();

        Some(GitInfo {
            branch,
            short_commit,
            is_dirty,
        })
    }
}

// ---------------------------------------------------------------------------
// Tool Execution Log Records
// ---------------------------------------------------------------------------

/// A sanitized record of a tool execution during the session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolExecutionRecord {
    /// Tool call identifier.
    pub call_id: String,
    /// Name of the invoked tool (e.g. "file_read", "grep", "bash").
    pub tool_name: String,
    /// Sanitized JSON or textual arguments passed to the tool.
    pub arguments: String,
    /// Sanitized output returned by the tool.
    pub output: String,
    /// Original size of the tool output in bytes before truncation.
    pub output_bytes: usize,
    /// Whether the output was truncated due to length constraints.
    pub is_truncated: bool,
    /// Whether the tool call completed successfully (inferred from output/errors).
    pub success: bool,
    /// Turn index in the conversation where this tool was invoked.
    pub turn_index: usize,
}

// ---------------------------------------------------------------------------
// Sanitized Conversation Snippet
// ---------------------------------------------------------------------------

/// A sanitized summary of a conversation message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SanitizedTranscriptMessage {
    /// Turn number.
    pub turn: usize,
    /// Role name ("user", "assistant", "system", "tool").
    pub role: String,
    /// Sanitized text snippet of the message content.
    pub content: String,
    /// Number of tool calls requested in this message (for assistant messages).
    pub tool_calls_count: usize,
}

// ---------------------------------------------------------------------------
// Session Metadata
// ---------------------------------------------------------------------------

/// Sanitized session metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMetadata {
    /// Session unique identifier.
    pub id: Uuid,
    /// Optional session title or derived heading.
    pub title: Option<String>,
    /// Active LLM model identifier.
    pub active_model: String,
    /// RFC 3339 creation timestamp.
    pub created_at: String,
    /// RFC 3339 last updated timestamp.
    pub updated_at: String,
    /// Sanitized working directory path.
    pub working_dir: Option<String>,
    /// Total count of messages in this session.
    pub total_messages: usize,
    /// Count of user messages.
    pub user_messages: usize,
    /// Count of assistant messages.
    pub assistant_messages: usize,
    /// Count of tool response messages.
    pub tool_messages: usize,
    /// Count of system prompt messages.
    pub system_messages: usize,
    /// Accumulated token usage statistics.
    pub token_stats: crate::agent::session::TokenStats,
    /// Sanitized session metadata key-value pairs.
    pub metadata: HashMap<String, String>,
}

// ---------------------------------------------------------------------------
// Trace Export Configuration Options
// ---------------------------------------------------------------------------

/// Configuration options controlling diagnostic trace generation.
#[derive(Debug, Clone)]
pub struct TraceExportOptions {
    /// Whether to include sanitized conversation transcript snippets in the trace.
    pub include_transcript: bool,
    /// Maximum character length per transcript snippet before truncation.
    pub max_transcript_chars: usize,
    /// Maximum character length per tool output before truncation.
    pub max_tool_output_chars: usize,
    /// Maximum number of recent tool executions to include (None for all).
    pub max_tool_records: Option<usize>,
    /// Optional user notes or issue description to prepend to the trace.
    pub notes: Option<String>,
    /// Custom sensitive strings to redact.
    pub custom_sensitive_strings: Vec<String>,
    /// Custom regex patterns to redact.
    pub custom_patterns: Vec<String>,
}

impl Default for TraceExportOptions {
    fn default() -> Self {
        Self {
            include_transcript: true,
            max_transcript_chars: 1000,
            max_tool_output_chars: 2048,
            max_tool_records: Some(50),
            notes: None,
            custom_sensitive_strings: Vec::new(),
            custom_patterns: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Complete Diagnostic Trace
// ---------------------------------------------------------------------------

/// Complete, privacy-safe diagnostic trace structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticTrace {
    /// Unique identifier for this diagnostic trace.
    pub trace_id: Uuid,
    /// Timestamp when this diagnostic trace was captured (RFC 3339).
    pub generated_at: String,
    /// Sanitized session metadata.
    pub session_metadata: SessionMetadata,
    /// Safe system environment information.
    pub system_info: SystemInfo,
    /// Audit log of privacy redactions performed.
    pub redaction_audit: RedactionAudit,
    /// Chronological list of sanitized tool executions.
    pub tool_executions: Vec<ToolExecutionRecord>,
    /// Sanitized conversation history snippet.
    pub transcript: Vec<SanitizedTranscriptMessage>,
    /// Optional user notes or diagnostic context.
    pub notes: Option<String>,
}

impl DiagnosticTrace {
    /// Collects a diagnostic trace from a session and optional runner configuration.
    pub fn collect(
        session: &Session,
        runner: Option<&AgentRunner>,
        options: Option<&TraceExportOptions>,
    ) -> Self {
        let opts = options.cloned().unwrap_or_default();
        let mut audit = RedactionAudit::default();

        // 1. Build redactor with known secrets from Config if available
        let mut redactor = TraceRedactor::new();
        if let Some(r) = runner {
            let cfg = r.config();
            let mut secrets = Vec::new();
            if let Some(k) = &cfg.deepseek_api_key {
                secrets.push(k.as_str());
            }
            if let Some(k) = &cfg.anthropic_api_key {
                secrets.push(k.as_str());
            }
            if let Some(k) = &cfg.openai_api_key {
                secrets.push(k.as_str());
            }
            if let Some(k) = &cfg.xai_api_key {
                secrets.push(k.as_str());
            }
            if let Some(k) = &cfg.openrouter_api_key {
                secrets.push(k.as_str());
            }
            redactor = redactor.with_known_secrets(&secrets);
        }

        // Add custom sensitive strings and patterns
        for s in &opts.custom_sensitive_strings {
            redactor = redactor.with_known_secrets(&[s.as_str()]);
        }
        for pat in &opts.custom_patterns {
            if let Ok(r) = redactor.clone().with_custom_pattern(pat) {
                redactor = r;
            }
        }

        // 2. Collect System Info
        let system_info = SystemInfo::collect(runner.map(|r| r.config()), &redactor, &mut audit);

        // 3. Collect Session Metadata
        let mut user_messages = 0;
        let mut assistant_messages = 0;
        let mut tool_messages = 0;
        let mut system_messages = 0;

        for msg in session.messages() {
            match msg.role {
                Role::User => user_messages += 1,
                Role::Assistant => assistant_messages += 1,
                Role::Tool => tool_messages += 1,
                Role::System => system_messages += 1,
            }
        }

        let title = session.title().map(|t| redactor.redact(t, &mut audit));
        let working_dir = session
            .working_dir()
            .map(|p| redactor.redact(&p.to_string_lossy(), &mut audit));

        let mut sanitized_metadata = HashMap::new();
        for (k, v) in session.metadata() {
            let clean_k = redactor.redact(k, &mut audit);
            let clean_v = redactor.redact(v, &mut audit);
            sanitized_metadata.insert(clean_k, clean_v);
        }

        let session_metadata = SessionMetadata {
            id: session.id(),
            title,
            active_model: session.active_model().to_string(),
            created_at: session.created_at().to_string(),
            updated_at: session.updated_at().to_string(),
            working_dir,
            total_messages: session.total_messages(),
            user_messages,
            assistant_messages,
            tool_messages,
            system_messages,
            token_stats: *session.token_stats(),
            metadata: sanitized_metadata,
        };

        // 4. Extract and sanitize tool execution logs from messages
        let mut tool_executions = Vec::new();
        let mut current_turn = 0;

        let messages = session.messages();
        for (idx, msg) in messages.iter().enumerate() {
            if msg.role == Role::User {
                current_turn += 1;
            }

            if msg.role == Role::Assistant {
                if let Some(tool_calls) = &msg.tool_calls {
                    for tc in tool_calls {
                        // Find matching Tool result message
                        let mut tool_output = String::new();
                        let mut found_result = false;

                        for later_msg in &messages[idx + 1..] {
                            if later_msg.role == Role::Tool {
                                if let Some(id) = &later_msg.tool_call_id {
                                    if id == &tc.id {
                                        tool_output = later_msg.content.clone();
                                        found_result = true;
                                        break;
                                    }
                                }
                            }
                        }

                        let raw_output = if found_result {
                            tool_output
                        } else {
                            "[In-flight or unrecorded tool result]".to_string()
                        };

                        let output_bytes = raw_output.len();
                        let success = !raw_output.trim_start().to_lowercase().starts_with("error")
                            && !raw_output.trim_start().to_lowercase().starts_with("failed");

                        // Sanitize arguments
                        let clean_args = redactor.redact(&tc.arguments, &mut audit);

                        // Pretty format arguments if valid JSON
                        let formatted_args =
                            match serde_json::from_str::<serde_json::Value>(&clean_args) {
                                Ok(v) => serde_json::to_string_pretty(&v).unwrap_or(clean_args),
                                Err(_) => clean_args,
                            };

                        // Sanitize and cap output
                        let clean_output = redactor.redact(&raw_output, &mut audit);
                        let is_truncated =
                            clean_output.chars().count() > opts.max_tool_output_chars;
                        let truncated_output = if is_truncated {
                            let head: String = clean_output
                                .chars()
                                .take(opts.max_tool_output_chars)
                                .collect();
                            format!(
                                "{}\n\n... [Output truncated: {} bytes total, showing first {} characters]",
                                head, output_bytes, opts.max_tool_output_chars
                            )
                        } else {
                            clean_output
                        };

                        tool_executions.push(ToolExecutionRecord {
                            call_id: tc.id.clone(),
                            tool_name: tc.name.clone(),
                            arguments: formatted_args,
                            output: truncated_output,
                            output_bytes,
                            is_truncated,
                            success,
                            turn_index: current_turn,
                        });
                    }
                }
            }
        }

        // Limit tool records if requested
        if let Some(max_recs) = opts.max_tool_records {
            if tool_executions.len() > max_recs {
                let start = tool_executions.len() - max_recs;
                tool_executions = tool_executions[start..].to_vec();
            }
        }

        // 5. Build sanitized transcript snippets if enabled
        let mut transcript = Vec::new();
        if opts.include_transcript {
            let mut turn = 0;
            for msg in session.messages() {
                if msg.role == Role::User {
                    turn += 1;
                }

                let role_str = match msg.role {
                    Role::User => "user",
                    Role::Assistant => "assistant",
                    Role::Tool => "tool",
                    Role::System => "system",
                };

                let clean_content = redactor.redact(&msg.content, &mut audit);
                let content_snippet = if clean_content.chars().count() > opts.max_transcript_chars {
                    let head: String = clean_content
                        .chars()
                        .take(opts.max_transcript_chars)
                        .collect();
                    format!("{}... [truncated]", head)
                } else {
                    clean_content
                };

                let tool_calls_count = msg.tool_calls.as_ref().map(|tc| tc.len()).unwrap_or(0);

                transcript.push(SanitizedTranscriptMessage {
                    turn,
                    role: role_str.to_string(),
                    content: content_snippet,
                    tool_calls_count,
                });
            }
        }

        let notes = opts.notes.map(|n| redactor.redact(&n, &mut audit));

        Self {
            trace_id: Uuid::new_v4(),
            generated_at: Utc::now().to_rfc3339(),
            session_metadata,
            system_info,
            redaction_audit: audit,
            tool_executions,
            transcript,
            notes,
        }
    }

    /// Renders the complete diagnostic trace into clean, professional Markdown.
    pub fn to_markdown(&self) -> String {
        let mut md = String::with_capacity(8192);

        // Header
        md.push_str("# Fusion Diagnostic Trace\n\n");
        md.push_str(&format!("> **Trace ID:** `{}`  \n", self.trace_id));
        md.push_str(&format!("> **Generated:** `{}`  \n", self.generated_at));
        md.push_str(&format!(
            "> **Fusion Version:** `v{}`  \n",
            self.system_info.fusion_version
        ));
        md.push_str(&format!(
            "> **Privacy Audit:** {}  \n\n",
            if self.redaction_audit.is_clean() {
                "✓ No sensitive data detected (clean)"
            } else {
                "🛡 Redactions applied (privacy-safe)"
            }
        ));

        // Optional User Notes
        if let Some(note) = &self.notes {
            md.push_str("## Diagnostic Notes\n\n");
            md.push_str(note);
            md.push_str("\n\n---\n\n");
        }

        // 1. Executive Summary
        md.push_str("## 1. Executive Summary\n\n");
        md.push_str("| Metric | Value |\n");
        md.push_str("| :--- | :--- |\n");
        md.push_str(&format!(
            "| **Session ID** | `{}` |\n",
            self.session_metadata.id
        ));
        md.push_str(&format!(
            "| **Title** | {} |\n",
            self.session_metadata
                .title
                .as_deref()
                .unwrap_or("*(untitled)*")
        ));
        md.push_str(&format!(
            "| **Active Model** | `{}` |\n",
            self.session_metadata.active_model
        ));
        md.push_str(&format!(
            "| **Total Turns** | {} |\n",
            self.session_metadata.token_stats.total_turns
        ));
        md.push_str(&format!(
            "| **Total Messages** | {} (User: {}, Assistant: {}, Tool: {}, System: {}) |\n",
            self.session_metadata.total_messages,
            self.session_metadata.user_messages,
            self.session_metadata.assistant_messages,
            self.session_metadata.tool_messages,
            self.session_metadata.system_messages
        ));
        md.push_str(&format!(
            "| **Total Tokens** | {} (Prompt: {}, Completion: {}) |\n",
            self.session_metadata.token_stats.total_tokens,
            self.session_metadata.token_stats.prompt_tokens,
            self.session_metadata.token_stats.completion_tokens
        ));
        md.push_str(&format!(
            "| **Tool Executions Logged** | {} |\n",
            self.tool_executions.len()
        ));
        md.push_str(&format!(
            "| **Privacy Redactions** | **{}** ({}) |\n\n",
            self.redaction_audit.total_redactions,
            self.redaction_audit.summary()
        ));

        // 2. Privacy & Redaction Audit
        md.push_str("## 2. Privacy & Redaction Audit\n\n");
        md.push_str("The diagnostic exporter automatically identifies and masks secrets, credentials, personal information, and host identities.\n\n");
        md.push_str("| Redaction Category | Count | Description |\n");
        md.push_str("| :--- | :---: | :--- |\n");
        md.push_str(&format!(
            "| **API Keys** | `{}` | LLM provider keys (Anthropic, OpenAI, DeepSeek, Google, etc.) |\n",
            self.redaction_audit.api_keys
        ));
        md.push_str(&format!(
            "| **Auth & Bearer Tokens** | `{}` | GitHub, Slack, and generic Authorization Bearer tokens |\n",
            self.redaction_audit.auth_tokens
        ));
        md.push_str(&format!(
            "| **Passwords & Secrets** | `{}` | Key-value credentials, CLI argument secrets, connection strings |\n",
            self.redaction_audit.passwords_and_secrets
        ));
        md.push_str(&format!(
            "| **Cryptographic Keys** | `{}` | PEM-formatted private keys (RSA, EC, ED25519) |\n",
            self.redaction_audit.private_keys
        ));
        md.push_str(&format!(
            "| **User & Home Paths** | `{}` | User home directories anonymized to `~` or `[USER]` |\n",
            self.redaction_audit.user_paths
        ));
        md.push_str(&format!(
            "| **Email Addresses** | `{}` | Personal and corporate email addresses masked |\n",
            self.redaction_audit.emails
        ));
        md.push_str(&format!(
            "| **IP Addresses** | `{}` | Network IP addresses masked (127.0.0.1 preserved) |\n",
            self.redaction_audit.ip_addresses
        ));
        if self.redaction_audit.custom > 0 {
            md.push_str(&format!(
                "| **Custom Redactions** | `{}` | User-supplied sensitive keywords or patterns |\n",
                self.redaction_audit.custom
            ));
        }
        md.push_str(&format!(
            "| **Total Redactions** | **`{}`** | **All detected sensitive items safely scrubbed** |\n\n",
            self.redaction_audit.total_redactions
        ));

        // 3. System Environment
        md.push_str("## 3. System Environment\n\n");
        md.push_str("| Environment Property | Value |\n");
        md.push_str("| :--- | :--- |\n");
        md.push_str(&format!(
            "| **Operating System** | `{}` (`{}`) |\n",
            self.system_info.os, self.system_info.os_family
        ));
        md.push_str(&format!(
            "| **Architecture** | `{}` ({}-bit) |\n",
            self.system_info.arch, self.system_info.pointer_width
        ));
        md.push_str(&format!(
            "| **CPU Cores** | {} |\n",
            self.system_info.cpu_cores
        ));
        md.push_str(&format!(
            "| **Fusion Version** | `v{}` |\n",
            self.system_info.fusion_version
        ));
        md.push_str(&format!(
            "| **Target Triple** | `{}` |\n",
            self.system_info.target_platform
        ));
        md.push_str(&format!(
            "| **Shell** | `{}` |\n",
            self.system_info.shell.as_deref().unwrap_or("unknown")
        ));
        md.push_str(&format!(
            "| **Terminal** | `{}` |\n",
            self.system_info.terminal.as_deref().unwrap_or("unknown")
        ));

        if let Some(git) = &self.system_info.git_info {
            let status = if git.is_dirty {
                "dirty (uncommitted changes)"
            } else {
                "clean"
            };
            md.push_str(&format!(
                "| **Git Status** | Branch `{}` at `{}` ({}) |\n",
                git.branch, git.short_commit, status
            ));
        } else {
            md.push_str("| **Git Status** | *(not a git repository or git unavailable)* |\n");
        }
        md.push_str("\n");

        // Provider Keys Readiness
        if !self.system_info.provider_keys_status.is_empty() {
            md.push_str("### Configured Providers\n\n");
            md.push_str("| Provider | Key Status |\n");
            md.push_str("| :--- | :--- |\n");
            for (p, configured) in &self.system_info.provider_keys_status {
                let badge = if *configured {
                    "✓ Configured"
                } else {
                    "✗ Missing Key"
                };
                md.push_str(&format!("| `{}` | {} |\n", p, badge));
            }
            md.push_str("\n");
        }

        // Safe Environment Variables
        if !self.system_info.safe_env_vars.is_empty() {
            md.push_str("### Safe Environment Variables\n\n");
            md.push_str("| Variable | Value |\n");
            md.push_str("| :--- | :--- |\n");
            for (k, v) in &self.system_info.safe_env_vars {
                md.push_str(&format!("| `{}` | `{}` |\n", k, v));
            }
            md.push_str("\n");
        }

        // 4. Detailed Token Statistics
        md.push_str("## 4. Token Usage & Cost Profile\n\n");
        md.push_str("| Metric | Token Count |\n");
        md.push_str("| :--- | :---: |\n");
        md.push_str(&format!(
            "| **Prompt Tokens** | `{}` |\n",
            self.session_metadata.token_stats.prompt_tokens
        ));
        md.push_str(&format!(
            "| **Completion Tokens** | `{}` |\n",
            self.session_metadata.token_stats.completion_tokens
        ));
        md.push_str(&format!(
            "| **Total Tokens** | `{}` |\n",
            self.session_metadata.token_stats.total_tokens
        ));
        md.push_str(&format!(
            "| **Cache Read Tokens** | `{}` |\n",
            self.session_metadata.token_stats.cache_read_tokens
        ));
        md.push_str(&format!(
            "| **Cache Write Tokens** | `{}` |\n",
            self.session_metadata.token_stats.cache_write_tokens
        ));
        md.push_str(&format!(
            "| **Total Turns** | `{}` |\n\n",
            self.session_metadata.token_stats.total_turns
        ));

        // 5. Recent Tool Executions
        md.push_str(&format!(
            "## 5. Tool Execution Logs ({} recorded)\n\n",
            self.tool_executions.len()
        ));

        if self.tool_executions.is_empty() {
            md.push_str("*(No tool executions recorded in this session)*\n\n");
        } else {
            for (i, tool) in self.tool_executions.iter().enumerate() {
                let status_icon = if tool.success {
                    "✓ Success"
                } else {
                    "✗ Error/Failed"
                };
                md.push_str(&format!(
                    "### Tool #{}: `{}` ({})\n\n",
                    i + 1,
                    tool.tool_name,
                    status_icon
                ));
                md.push_str(&format!("- **Call ID:** `{}`\n", tool.call_id));
                md.push_str(&format!("- **Turn:** {}\n", tool.turn_index));
                md.push_str(&format!(
                    "- **Output Size:** {} bytes{}\n\n",
                    tool.output_bytes,
                    if tool.is_truncated {
                        " *(truncated)*"
                    } else {
                        ""
                    }
                ));

                md.push_str("**Arguments:**\n");
                md.push_str("```json\n");
                md.push_str(&tool.arguments);
                md.push_str("\n```\n\n");

                md.push_str("**Result Output:**\n");
                md.push_str("```\n");
                md.push_str(&tool.output);
                md.push_str("\n```\n\n");
            }
        }

        // 6. Sanitized Conversation Transcript
        if !self.transcript.is_empty() {
            md.push_str(&format!(
                "## 6. Conversation Transcript ({} messages)\n\n",
                self.transcript.len()
            ));

            for (i, item) in self.transcript.iter().enumerate() {
                let role_badge = match item.role.as_str() {
                    "user" => "👤 User",
                    "assistant" => "🤖 Assistant",
                    "tool" => "⚙️ Tool",
                    "system" => "📋 System",
                    other => other,
                };

                let tool_info = if item.tool_calls_count > 0 {
                    format!(
                        " (requested {} tool call{})",
                        item.tool_calls_count,
                        if item.tool_calls_count == 1 { "" } else { "s" }
                    )
                } else {
                    String::new()
                };

                md.push_str(&format!(
                    "#### Message #{}: {} [Turn {}]{}\n\n",
                    i + 1,
                    role_badge,
                    item.turn,
                    tool_info
                ));
                md.push_str("```\n");
                md.push_str(&item.content);
                md.push_str("\n```\n\n");
            }
        }

        md.push_str("---\n*Trace generated by Fusion v");
        md.push_str(&self.system_info.fusion_version);
        md.push_str(" privacy-safe diagnostic engine.*\n");

        md
    }

    /// Saves the diagnostic trace to a markdown file on disk.
    /// If `path` is None, saves to standard location: `~/.fusion/traces/trace_<session_id>_<timestamp>.md`.
    pub fn save_to_file(&self, path: Option<&Path>) -> anyhow::Result<PathBuf> {
        let destination = match path {
            Some(p) => p.to_path_buf(),
            None => {
                let dir = traces_dir();
                if !dir.exists() {
                    fs::create_dir_all(&dir)?;
                }
                let safe_timestamp = self.generated_at.replace(':', "-").replace('.', "-");
                dir.join(format!(
                    "trace_{}_{}.md",
                    self.session_metadata.id, safe_timestamp
                ))
            }
        };

        if let Some(parent) = destination.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)?;
            }
        }

        let markdown = self.to_markdown();
        fs::write(&destination, markdown)?;

        Ok(destination)
    }
}

// ---------------------------------------------------------------------------
// Public Helper Functions
// ---------------------------------------------------------------------------

/// Returns the default directory where diagnostic traces are stored: `~/.fusion/traces`.
pub fn traces_dir() -> PathBuf {
    Config::config_dir().join("traces")
}

/// Generates a diagnostic trace from an active session and optional runner configuration.
pub fn generate_trace(
    session: &Session,
    runner: Option<&AgentRunner>,
    options: Option<&TraceExportOptions>,
) -> DiagnosticTrace {
    DiagnosticTrace::collect(session, runner, options)
}

/// Exports the diagnostic trace directly as a sanitized Markdown string.
pub fn export_trace_markdown(
    session: &Session,
    runner: Option<&AgentRunner>,
    options: Option<&TraceExportOptions>,
) -> (String, RedactionAudit) {
    let trace = generate_trace(session, runner, options);
    let audit = trace.redaction_audit.clone();
    (trace.to_markdown(), audit)
}

/// Saves the diagnostic trace to a file, returning the saved path and redaction audit report.
pub fn save_trace_file(
    session: &Session,
    runner: Option<&AgentRunner>,
    path: Option<&Path>,
    options: Option<&TraceExportOptions>,
) -> anyhow::Result<(PathBuf, RedactionAudit)> {
    let trace = generate_trace(session, runner, options);
    let audit = trace.redaction_audit.clone();
    let saved_path = trace.save_to_file(path)?;
    Ok((saved_path, audit))
}

/// Interactive slash command handler for `/trace [path]`.
/// Collects session metadata, system info, recent tool execution logs, and redactions,
/// writing the clean Markdown trace to the specified path or `~/.fusion/traces/`.
pub fn handle_trace_command(
    path: Option<&str>,
    runner: &AgentRunner,
    session: &Session,
) -> Option<PathBuf> {
    let custom_path = path.map(PathBuf::from);
    match save_trace_file(session, Some(runner), custom_path.as_deref(), None) {
        Ok((saved_path, audit)) => {
            let status_line = if audit.is_clean() {
                "✓ No sensitive data detected (clean trace)".to_string()
            } else {
                format!(
                    "🛡 Privacy audit: {} sensitive items redacted ({})",
                    audit.total_redactions,
                    audit.summary()
                )
            };

            println!("\x1b[1;32m✓\x1b[0m Diagnostic trace successfully generated!");
            println!(
                "  \x1b[1;37mFile:\x1b[0m     \x1b[36m{}\x1b[0m",
                saved_path.display()
            );
            println!("  \x1b[1;37mSession:\x1b[0m  {}", session.id());
            println!("  \x1b[1;37mModel:\x1b[0m    {}", session.active_model());
            println!("  \x1b[1;37mStatus:\x1b[0m   {}\n", status_line);

            Some(saved_path)
        }
        Err(e) => {
            eprintln!("\x1b[1;31m✗\x1b[0m Failed to generate diagnostic trace: {e}\n");
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Execution Trace Logging & Observability Engine
// ---------------------------------------------------------------------------

/// Categorization of events tracked by the structured execution trace logger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceEventType {
    /// Full agent turn cycle (user prompt -> reasoning -> tools -> response).
    AgentTurn,
    /// Invocation and execution of an agent tool.
    ToolInvocation,
    /// Direct API request to an LLM provider.
    LlmRequest,
    /// Entire multi-turn agent session lifecycle.
    AgentSession,
    /// Delegated subagent lifecycle execution.
    SubagentExecution,
    /// Context window compaction or summarization pass.
    ContextCompaction,
    /// Agent memory search or retrieval operation.
    MemoryRetrieval,
    /// Generic or user-defined custom span.
    Custom,
}

/// Execution status of a trace span.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", content = "message", rename_all = "snake_case")]
pub enum TraceStatus {
    /// Operation completed successfully.
    Ok,
    /// Operation failed with an error message.
    Error { message: String },
    /// Operation was cancelled before completion.
    Cancelled,
    /// Operation is actively executing and not yet finalized.
    InProgress,
    /// Status is unset.
    Unset,
}

impl TraceStatus {
    /// Returns `true` if the status represents success.
    pub fn is_ok(&self) -> bool {
        matches!(self, Self::Ok)
    }

    /// Returns `true` if the status represents an error condition.
    pub fn is_error(&self) -> bool {
        matches!(self, Self::Error { .. })
    }

    /// Returns the error message string if this status represents an error.
    pub fn error_message(&self) -> Option<&str> {
        match self {
            Self::Error { message } => Some(message.as_str()),
            _ => None,
        }
    }
}

/// Detailed token metrics recorded for an agent turn or LLM request.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceTokenMetrics {
    /// Prompt / input tokens consumed.
    #[serde(default)]
    pub prompt_tokens: u64,
    /// Completion / output tokens generated.
    #[serde(default)]
    pub completion_tokens: u64,
    /// Total tokens (prompt + completion).
    #[serde(default)]
    pub total_tokens: u64,
    /// Cached tokens read from provider context cache.
    #[serde(default)]
    pub cache_read_tokens: u64,
    /// Cached tokens written to provider context cache.
    #[serde(default)]
    pub cache_write_tokens: u64,
}

impl From<TokenStats> for TraceTokenMetrics {
    fn from(stats: TokenStats) -> Self {
        Self {
            prompt_tokens: stats.prompt_tokens,
            completion_tokens: stats.completion_tokens,
            total_tokens: stats.total_tokens,
            cache_read_tokens: stats.cache_read_tokens,
            cache_write_tokens: stats.cache_write_tokens,
        }
    }
}

/// Specific metadata for tool invocation spans.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceToolMetadata {
    /// Name of the invoked tool (e.g. "grep", "file_read", "bash").
    pub tool_name: String,
    /// Call ID associated with the tool invocation.
    pub call_id: String,
    /// Sanitized summary of input arguments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments_summary: Option<String>,
    /// Length of the tool output in bytes.
    #[serde(default)]
    pub output_bytes: usize,
    /// Whether output was truncated.
    #[serde(default)]
    pub is_truncated: bool,
    /// Whether tool execution produced an error.
    #[serde(default)]
    pub is_error: bool,
}

/// Specific metadata for LLM provider request spans.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TraceLlmMetadata {
    /// Model identifier requested.
    pub model: String,
    /// Provider name (e.g. "anthropic", "openai", "deepseek").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Sampling temperature if set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    /// Finish reason returned by provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    /// Time-to-first-token in milliseconds if measured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_token_latency_ms: Option<f64>,
}

/// In-span timestamped log event or annotation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TraceLogEvent {
    /// Event name or description.
    pub name: String,
    /// Timestamp in RFC 3339 format.
    pub timestamp: String,
    /// Timestamp in Unix nanoseconds.
    pub timestamp_unix_nano: u64,
    /// Key-value attributes associated with the event.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub attributes: HashMap<String, serde_json::Value>,
}

/// Canonical execution trace record serialized into JSONL.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TraceRecord {
    /// Unique record identifier.
    pub record_id: Uuid,
    /// OpenTelemetry compatible 32-hex trace ID.
    pub trace_id: String,
    /// OpenTelemetry compatible 16-hex span ID.
    pub span_id: String,
    /// Optional parent span ID for trace hierarchy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_span_id: Option<String>,
    /// Session unique identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Turn index within the conversation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_index: Option<usize>,
    /// Type of execution event.
    pub event_type: TraceEventType,
    /// Descriptive name of the span.
    pub name: String,
    /// Span start timestamp in RFC 3339.
    pub start_time: String,
    /// Span start timestamp in Unix nanoseconds.
    pub start_time_unix_nano: u64,
    /// Span end timestamp in RFC 3339.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_time: Option<String>,
    /// Span end timestamp in Unix nanoseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_time_unix_nano: Option<u64>,
    /// Measured duration in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<f64>,
    /// Execution status.
    pub status: TraceStatus,
    /// Token metrics for turns and LLM calls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_metrics: Option<TraceTokenMetrics>,
    /// Tool execution metadata if applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_metadata: Option<TraceToolMetadata>,
    /// LLM request metadata if applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm_metadata: Option<TraceLlmMetadata>,
    /// Generic structured attributes.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub attributes: HashMap<String, serde_json::Value>,
    /// Events or log entries emitted during this span.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<TraceLogEvent>,
}

/// Type alias for ExecutionSpan compatibility.
pub type ExecutionSpan = TraceRecord;

impl TraceRecord {
    /// Serializes this trace record to a single JSON line.
    pub fn to_jsonl_line(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// Deserializes a trace record from a JSON line string.
    pub fn from_jsonl_line(line: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(line.trim())
    }

    /// Converts this record into a standard OpenTelemetry (OTLP) Span.
    pub fn to_otel_span(&self) -> OtelSpan {
        let kind = match self.event_type {
            TraceEventType::ToolInvocation | TraceEventType::LlmRequest => {
                OtelSpanKind::Client as i32
            }
            TraceEventType::AgentSession | TraceEventType::AgentTurn => OtelSpanKind::Server as i32,
            _ => OtelSpanKind::Internal as i32,
        };

        let status = match &self.status {
            TraceStatus::Ok => OtelStatus {
                code: OtelStatusCode::Ok as i32,
                message: None,
            },
            TraceStatus::Error { message } => OtelStatus {
                code: OtelStatusCode::Error as i32,
                message: Some(message.clone()),
            },
            _ => OtelStatus {
                code: OtelStatusCode::Unset as i32,
                message: None,
            },
        };

        let mut attributes = Vec::new();

        // Semantic attribute conventions
        attributes.push(OtelKeyValue {
            key: "fusion.event_type".to_string(),
            value: format!("{:?}", self.event_type).to_lowercase().into(),
        });

        if let Some(session_id) = &self.session_id {
            attributes.push(OtelKeyValue {
                key: "session.id".to_string(),
                value: session_id.clone().into(),
            });
        }

        if let Some(turn) = self.turn_index {
            attributes.push(OtelKeyValue {
                key: "agent.turn_index".to_string(),
                value: (turn as i64).into(),
            });
        }

        if let Some(dur) = self.duration_ms {
            attributes.push(OtelKeyValue {
                key: "duration_ms".to_string(),
                value: dur.into(),
            });
        }

        if let Some(toks) = &self.token_metrics {
            attributes.push(OtelKeyValue {
                key: "gen_ai.usage.prompt_tokens".to_string(),
                value: toks.prompt_tokens.into(),
            });
            attributes.push(OtelKeyValue {
                key: "gen_ai.usage.completion_tokens".to_string(),
                value: toks.completion_tokens.into(),
            });
            attributes.push(OtelKeyValue {
                key: "gen_ai.usage.total_tokens".to_string(),
                value: toks.total_tokens.into(),
            });
            if toks.cache_read_tokens > 0 {
                attributes.push(OtelKeyValue {
                    key: "gen_ai.usage.cache_read_tokens".to_string(),
                    value: toks.cache_read_tokens.into(),
                });
            }
            if toks.cache_write_tokens > 0 {
                attributes.push(OtelKeyValue {
                    key: "gen_ai.usage.cache_write_tokens".to_string(),
                    value: toks.cache_write_tokens.into(),
                });
            }
        }

        if let Some(tool) = &self.tool_metadata {
            attributes.push(OtelKeyValue {
                key: "tool.name".to_string(),
                value: tool.tool_name.clone().into(),
            });
            attributes.push(OtelKeyValue {
                key: "tool.call_id".to_string(),
                value: tool.call_id.clone().into(),
            });
            attributes.push(OtelKeyValue {
                key: "tool.output_bytes".to_string(),
                value: (tool.output_bytes as i64).into(),
            });
            attributes.push(OtelKeyValue {
                key: "tool.is_error".to_string(),
                value: tool.is_error.into(),
            });
            if let Some(args) = &tool.arguments_summary {
                attributes.push(OtelKeyValue {
                    key: "tool.arguments".to_string(),
                    value: args.clone().into(),
                });
            }
        }

        if let Some(llm) = &self.llm_metadata {
            attributes.push(OtelKeyValue {
                key: "gen_ai.request.model".to_string(),
                value: llm.model.clone().into(),
            });
            if let Some(provider) = &llm.provider {
                attributes.push(OtelKeyValue {
                    key: "gen_ai.system".to_string(),
                    value: provider.clone().into(),
                });
            }
            if let Some(temp) = llm.temperature {
                attributes.push(OtelKeyValue {
                    key: "gen_ai.request.temperature".to_string(),
                    value: (temp as f64).into(),
                });
            }
            if let Some(reason) = &llm.finish_reason {
                attributes.push(OtelKeyValue {
                    key: "gen_ai.response.finish_reasons".to_string(),
                    value: reason.clone().into(),
                });
            }
            if let Some(ttft) = llm.first_token_latency_ms {
                attributes.push(OtelKeyValue {
                    key: "gen_ai.response.time_to_first_token_ms".to_string(),
                    value: ttft.into(),
                });
            }
        }

        for (k, v) in &self.attributes {
            attributes.push(OtelKeyValue {
                key: k.clone(),
                value: OtelAnyValue::from(v),
            });
        }

        let events = self
            .events
            .iter()
            .map(|e| {
                let mut ev_attrs = Vec::new();
                for (k, v) in &e.attributes {
                    ev_attrs.push(OtelKeyValue {
                        key: k.clone(),
                        value: OtelAnyValue::from(v),
                    });
                }
                OtelEvent {
                    time_unix_nano: e.timestamp_unix_nano,
                    name: e.name.clone(),
                    attributes: ev_attrs,
                }
            })
            .collect();

        let end_nano = self.end_time_unix_nano.unwrap_or(self.start_time_unix_nano);

        OtelSpan {
            trace_id: self.trace_id.clone(),
            span_id: self.span_id.clone(),
            parent_span_id: self.parent_span_id.clone(),
            name: self.name.clone(),
            kind,
            start_time_unix_nano: self.start_time_unix_nano,
            end_time_unix_nano: end_nano,
            attributes,
            status,
            events,
        }
    }
}

// ---------------------------------------------------------------------------
// OpenTelemetry (OTLP) Compatible Data Structures
// ---------------------------------------------------------------------------

/// OpenTelemetry standard SpanKind enumeration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(i32)]
pub enum OtelSpanKind {
    Unspecified = 0,
    Internal = 1,
    Server = 2,
    Client = 3,
    Producer = 4,
    Consumer = 5,
}

/// OpenTelemetry standard StatusCode enumeration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(i32)]
pub enum OtelStatusCode {
    Unset = 0,
    Ok = 1,
    Error = 2,
}

/// OpenTelemetry standard Status message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OtelStatus {
    pub code: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// OpenTelemetry generic attribute value container.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OtelAnyValue {
    StringValue {
        string_value: String,
    },
    BoolValue {
        bool_value: bool,
    },
    IntValue {
        int_value: i64,
    },
    DoubleValue {
        double_value: f64,
    },
    ArrayValue {
        array_value: Vec<OtelAnyValue>,
    },
    KvlistValue {
        kvlist_value: HashMap<String, OtelAnyValue>,
    },
}

impl From<&str> for OtelAnyValue {
    fn from(s: &str) -> Self {
        Self::StringValue {
            string_value: s.to_string(),
        }
    }
}

impl From<String> for OtelAnyValue {
    fn from(s: String) -> Self {
        Self::StringValue { string_value: s }
    }
}

impl From<bool> for OtelAnyValue {
    fn from(b: bool) -> Self {
        Self::BoolValue { bool_value: b }
    }
}

impl From<i64> for OtelAnyValue {
    fn from(i: i64) -> Self {
        Self::IntValue { int_value: i }
    }
}

impl From<u64> for OtelAnyValue {
    fn from(u: u64) -> Self {
        Self::IntValue {
            int_value: u as i64,
        }
    }
}

impl From<f64> for OtelAnyValue {
    fn from(f: f64) -> Self {
        Self::DoubleValue { double_value: f }
    }
}

impl From<&serde_json::Value> for OtelAnyValue {
    fn from(v: &serde_json::Value) -> Self {
        match v {
            serde_json::Value::Null => Self::StringValue {
                string_value: "null".to_string(),
            },
            serde_json::Value::Bool(b) => Self::BoolValue { bool_value: *b },
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Self::IntValue { int_value: i }
                } else if let Some(f) = n.as_f64() {
                    Self::DoubleValue { double_value: f }
                } else {
                    Self::StringValue {
                        string_value: n.to_string(),
                    }
                }
            }
            serde_json::Value::String(s) => Self::StringValue {
                string_value: s.clone(),
            },
            serde_json::Value::Array(arr) => Self::ArrayValue {
                array_value: arr.iter().map(OtelAnyValue::from).collect(),
            },
            serde_json::Value::Object(map) => {
                let mut kv = HashMap::new();
                for (k, val) in map {
                    kv.insert(k.clone(), OtelAnyValue::from(val));
                }
                Self::KvlistValue { kvlist_value: kv }
            }
        }
    }
}

/// OpenTelemetry key-value pair.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OtelKeyValue {
    pub key: String,
    pub value: OtelAnyValue,
}

/// OpenTelemetry in-span event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OtelEvent {
    pub time_unix_nano: u64,
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attributes: Vec<OtelKeyValue>,
}

/// OpenTelemetry standard span representation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OtelSpan {
    pub trace_id: String,
    pub span_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_span_id: Option<String>,
    pub name: String,
    pub kind: i32,
    pub start_time_unix_nano: u64,
    pub end_time_unix_nano: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attributes: Vec<OtelKeyValue>,
    pub status: OtelStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<OtelEvent>,
}

/// OpenTelemetry instrumentation scope metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OtelInstrumentationScope {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// OpenTelemetry collection of spans for an instrumentation scope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OtelScopeSpans {
    pub scope: OtelInstrumentationScope,
    pub spans: Vec<OtelSpan>,
}

/// OpenTelemetry resource descriptor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OtelResource {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attributes: Vec<OtelKeyValue>,
}

/// OpenTelemetry resource spans payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OtelResourceSpans {
    pub resource: OtelResource,
    pub scope_spans: Vec<OtelScopeSpans>,
}

/// OpenTelemetry full export payload compliant with OTLP standard.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OtelExportPayload {
    pub resource_spans: Vec<OtelResourceSpans>,
}

// ---------------------------------------------------------------------------
// Trace Query, Filter & Analytics Engine
// ---------------------------------------------------------------------------

/// Status filter mode for trace queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TraceStatusFilter {
    #[default]
    All,
    SuccessOnly,
    ErrorsOnly,
}

/// Rich query filter for searching and analyzing trace records.
#[derive(Debug, Clone, Default)]
pub struct TraceQuery {
    /// Match by exact session ID.
    pub session_id: Option<String>,
    /// Match by tool name.
    pub tool_name: Option<String>,
    /// Match by execution status (all, success, errors only).
    pub status_filter: TraceStatusFilter,
    /// Match by specific event type.
    pub event_type: Option<TraceEventType>,
    /// Match if span name contains substring.
    pub name_contains: Option<String>,
    /// Match spans with duration >= min_duration_ms.
    pub min_duration_ms: Option<f64>,
    /// Match spans with duration <= max_duration_ms.
    pub max_duration_ms: Option<f64>,
    /// Match spans starting on or after Unix nanoseconds timestamp.
    pub since_unix_nano: Option<u64>,
    /// Match spans starting on or before Unix nanoseconds timestamp.
    pub until_unix_nano: Option<u64>,
    /// Match spans with total tokens >= min_total_tokens.
    pub min_total_tokens: Option<u64>,
    /// Max number of records to return.
    pub limit: Option<usize>,
    /// Number of records to skip before returning.
    pub offset: Option<usize>,
}

impl TraceQuery {
    /// Creates a new empty query that matches all records.
    pub fn new() -> Self {
        Self::default()
    }

    /// Filters by session identifier.
    pub fn with_session_id(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }

    /// Filters by tool name.
    pub fn with_tool_name(mut self, tool_name: impl Into<String>) -> Self {
        self.tool_name = Some(tool_name.into());
        self
    }

    /// Sets the status filter mode.
    pub fn with_status(mut self, filter: TraceStatusFilter) -> Self {
        self.status_filter = filter;
        self
    }

    /// Filters for only error records.
    pub fn errors_only(mut self) -> Self {
        self.status_filter = TraceStatusFilter::ErrorsOnly;
        self
    }

    /// Filters for only successful records.
    pub fn success_only(mut self) -> Self {
        self.status_filter = TraceStatusFilter::SuccessOnly;
        self
    }

    /// Filters by specific event type.
    pub fn with_event_type(mut self, event_type: TraceEventType) -> Self {
        self.event_type = Some(event_type);
        self
    }

    /// Filters spans containing the specified name substring.
    pub fn with_name_contains(mut self, needle: impl Into<String>) -> Self {
        self.name_contains = Some(needle.into());
        self
    }

    /// Filters spans with minimum duration in milliseconds.
    pub fn with_min_duration(mut self, min_ms: f64) -> Self {
        self.min_duration_ms = Some(min_ms);
        self
    }

    /// Filters spans with maximum duration in milliseconds.
    pub fn with_max_duration(mut self, max_ms: f64) -> Self {
        self.max_duration_ms = Some(max_ms);
        self
    }

    /// Filters spans starting after or at given timestamp in Unix nanoseconds.
    pub fn with_since(mut self, since_nano: u64) -> Self {
        self.since_unix_nano = Some(since_nano);
        self
    }

    /// Filters spans starting before or at given timestamp in Unix nanoseconds.
    pub fn with_until(mut self, until_nano: u64) -> Self {
        self.until_unix_nano = Some(until_nano);
        self
    }

    /// Filters spans with at least the given total token count.
    pub fn with_min_total_tokens(mut self, min_tokens: u64) -> Self {
        self.min_total_tokens = Some(min_tokens);
        self
    }

    /// Limits the number of results.
    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Skips the first `offset` results.
    pub fn with_offset(mut self, offset: usize) -> Self {
        self.offset = Some(offset);
        self
    }

    /// Evaluates whether a trace record satisfies this query's filters.
    pub fn matches(&self, record: &TraceRecord) -> bool {
        if let Some(sid) = &self.session_id {
            if record.session_id.as_deref() != Some(sid.as_str()) {
                return false;
            }
        }

        if let Some(tool) = &self.tool_name {
            match &record.tool_metadata {
                Some(meta) if meta.tool_name == *tool => {}
                _ => return false,
            }
        }

        match self.status_filter {
            TraceStatusFilter::All => {}
            TraceStatusFilter::SuccessOnly => {
                if record.status.is_error() {
                    return false;
                }
            }
            TraceStatusFilter::ErrorsOnly => {
                if !record.status.is_error() {
                    return false;
                }
            }
        }

        if let Some(event_type) = self.event_type {
            if record.event_type != event_type {
                return false;
            }
        }

        if let Some(needle) = &self.name_contains {
            if !record.name.contains(needle) {
                return false;
            }
        }

        if let Some(min_d) = self.min_duration_ms {
            match record.duration_ms {
                Some(d) if d >= min_d => {}
                _ => return false,
            }
        }

        if let Some(max_d) = self.max_duration_ms {
            match record.duration_ms {
                Some(d) if d <= max_d => {}
                _ => return false,
            }
        }

        if let Some(since) = self.since_unix_nano {
            if record.start_time_unix_nano < since {
                return false;
            }
        }

        if let Some(until) = self.until_unix_nano {
            if record.start_time_unix_nano > until {
                return false;
            }
        }

        if let Some(min_toks) = self.min_total_tokens {
            match &record.token_metrics {
                Some(m) if m.total_tokens >= min_toks => {}
                _ => return false,
            }
        }

        true
    }
}

/// Aggregated performance and usage statistics for a specific tool.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ToolUsageStats {
    /// Total invocations of the tool.
    pub call_count: usize,
    /// Total error executions.
    pub error_count: usize,
    /// Total accumulated execution time in milliseconds.
    pub total_duration_ms: f64,
    /// Average execution time in milliseconds.
    pub avg_duration_ms: f64,
    /// Minimum execution time in milliseconds.
    pub min_duration_ms: f64,
    /// Maximum execution time in milliseconds.
    pub max_duration_ms: f64,
    /// Total payload output volume in bytes.
    pub total_output_bytes: usize,
    /// Tool success rate (0.0 to 1.0).
    pub success_rate: f64,
}

/// Comprehensive statistical analytics computed from a set of trace records.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TraceAnalytics {
    /// Total trace records analyzed.
    pub total_records: usize,
    /// Total turn spans recorded.
    pub total_turns: usize,
    /// Total tool invocation spans recorded.
    pub total_tool_calls: usize,
    /// Total LLM request spans recorded.
    pub total_llm_calls: usize,
    /// Total error spans recorded.
    pub total_errors: usize,
    /// Overall error rate (0.0 to 1.0).
    pub error_rate: f64,
    /// Total duration accumulated across all spans in milliseconds.
    pub total_duration_ms: f64,
    /// Average span duration in milliseconds.
    pub avg_duration_ms: f64,
    /// Minimum span duration in milliseconds.
    pub min_duration_ms: f64,
    /// Maximum span duration in milliseconds.
    pub max_duration_ms: f64,
    /// Median (p50) duration in milliseconds.
    pub p50_duration_ms: f64,
    /// 90th percentile (p90) duration in milliseconds.
    pub p90_duration_ms: f64,
    /// 95th percentile (p95) duration in milliseconds.
    pub p95_duration_ms: f64,
    /// 99th percentile (p99) duration in milliseconds.
    pub p99_duration_ms: f64,
    /// Accumulated token consumption statistics.
    pub total_tokens: TraceTokenMetrics,
    /// Per-tool usage breakdown statistics.
    pub tool_breakdown: HashMap<String, ToolUsageStats>,
}

/// High-performance streaming trace reader for querying and computing analytics.
pub struct TraceReader;

impl TraceReader {
    /// Reads all trace records from a JSONL file.
    pub fn read_file(path: &Path) -> anyhow::Result<Vec<TraceRecord>> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);
        Self::read_reader(reader)
    }

    /// Reads and filters trace records from a JSONL file using a `TraceQuery`.
    pub fn filter_file(path: &Path, filter: &TraceQuery) -> anyhow::Result<Vec<TraceRecord>> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);
        Self::filter_reader(reader, filter)
    }

    /// Reads all trace records from a generic buffered reader.
    pub fn read_reader<R: BufRead>(reader: R) -> anyhow::Result<Vec<TraceRecord>> {
        let mut records = Vec::new();
        for line_res in reader.lines() {
            let line = line_res?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Ok(record) = TraceRecord::from_jsonl_line(trimmed) {
                records.push(record);
            }
        }
        Ok(records)
    }

    /// Reads and filters trace records from a generic buffered reader.
    pub fn filter_reader<R: BufRead>(
        reader: R,
        filter: &TraceQuery,
    ) -> anyhow::Result<Vec<TraceRecord>> {
        let mut records = Vec::new();
        let mut skipped = 0;

        for line_res in reader.lines() {
            let line = line_res?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Ok(record) = TraceRecord::from_jsonl_line(trimmed) {
                if filter.matches(&record) {
                    if let Some(offset) = filter.offset {
                        if skipped < offset {
                            skipped += 1;
                            continue;
                        }
                    }
                    records.push(record);
                    if let Some(limit) = filter.limit {
                        if records.len() >= limit {
                            break;
                        }
                    }
                }
            }
        }
        Ok(records)
    }

    /// Computes aggregated analytics across a slice of trace records.
    pub fn compute_analytics(records: &[TraceRecord]) -> TraceAnalytics {
        if records.is_empty() {
            return TraceAnalytics::default();
        }

        let mut total_turns = 0;
        let mut total_tool_calls = 0;
        let mut total_llm_calls = 0;
        let mut total_errors = 0;
        let mut total_duration_ms = 0.0;
        let mut min_duration_ms = f64::MAX;
        let mut max_duration_ms = 0.0;
        let mut durations: Vec<f64> = Vec::with_capacity(records.len());

        let mut total_tokens = TraceTokenMetrics::default();
        let mut tool_data: HashMap<String, (usize, usize, f64, f64, f64, usize)> = HashMap::new();

        for r in records {
            match r.event_type {
                TraceEventType::AgentTurn => total_turns += 1,
                TraceEventType::ToolInvocation => total_tool_calls += 1,
                TraceEventType::LlmRequest => total_llm_calls += 1,
                _ => {}
            }

            if r.status.is_error() {
                total_errors += 1;
            }

            if let Some(dur) = r.duration_ms {
                durations.push(dur);
                total_duration_ms += dur;
                if dur < min_duration_ms {
                    min_duration_ms = dur;
                }
                if dur > max_duration_ms {
                    max_duration_ms = dur;
                }
            }

            if let Some(toks) = &r.token_metrics {
                total_tokens.prompt_tokens = total_tokens
                    .prompt_tokens
                    .saturating_add(toks.prompt_tokens);
                total_tokens.completion_tokens = total_tokens
                    .completion_tokens
                    .saturating_add(toks.completion_tokens);
                total_tokens.total_tokens =
                    total_tokens.total_tokens.saturating_add(toks.total_tokens);
                total_tokens.cache_read_tokens = total_tokens
                    .cache_read_tokens
                    .saturating_add(toks.cache_read_tokens);
                total_tokens.cache_write_tokens = total_tokens
                    .cache_write_tokens
                    .saturating_add(toks.cache_write_tokens);
            }

            if let Some(tool) = &r.tool_metadata {
                let dur = r.duration_ms.unwrap_or(0.0);
                let entry = tool_data.entry(tool.tool_name.clone()).or_insert((
                    0,
                    0,
                    0.0,
                    f64::MAX,
                    0.0,
                    0,
                ));
                entry.0 += 1; // count
                if tool.is_error || r.status.is_error() {
                    entry.1 += 1; // errors
                }
                entry.2 += dur; // total duration
                if dur < entry.3 {
                    entry.3 = dur; // min duration
                }
                if dur > entry.4 {
                    entry.4 = dur; // max duration
                }
                entry.5 += tool.output_bytes; // output bytes
            }
        }

        durations.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        let p50 = Self::percentile(&durations, 50.0);
        let p90 = Self::percentile(&durations, 90.0);
        let p95 = Self::percentile(&durations, 95.0);
        let p99 = Self::percentile(&durations, 99.0);

        let avg_dur = if !durations.is_empty() {
            total_duration_ms / (durations.len() as f64)
        } else {
            0.0
        };

        if min_duration_ms == f64::MAX {
            min_duration_ms = 0.0;
        }

        let mut tool_breakdown = HashMap::new();
        for (name, (cnt, errs, tot_dur, min_dur, max_dur, bytes)) in tool_data {
            let avg = if cnt > 0 { tot_dur / (cnt as f64) } else { 0.0 };
            let success_rate = if cnt > 0 {
                ((cnt - errs) as f64) / (cnt as f64)
            } else {
                1.0
            };
            tool_breakdown.insert(
                name,
                ToolUsageStats {
                    call_count: cnt,
                    error_count: errs,
                    total_duration_ms: tot_dur,
                    avg_duration_ms: avg,
                    min_duration_ms: if min_dur == f64::MAX { 0.0 } else { min_dur },
                    max_duration_ms: max_dur,
                    total_output_bytes: bytes,
                    success_rate,
                },
            );
        }

        let error_rate = if !records.is_empty() {
            (total_errors as f64) / (records.len() as f64)
        } else {
            0.0
        };

        TraceAnalytics {
            total_records: records.len(),
            total_turns,
            total_tool_calls,
            total_llm_calls,
            total_errors,
            error_rate,
            total_duration_ms,
            avg_duration_ms: avg_dur,
            min_duration_ms,
            max_duration_ms,
            p50_duration_ms: p50,
            p90_duration_ms: p90,
            p95_duration_ms: p95,
            p99_duration_ms: p99,
            total_tokens,
            tool_breakdown,
        }
    }

    fn percentile(sorted: &[f64], pct: f64) -> f64 {
        if sorted.is_empty() {
            return 0.0;
        }
        if sorted.len() == 1 {
            return sorted[0];
        }
        let rank = (pct / 100.0) * ((sorted.len() - 1) as f64);
        let lower = rank.floor() as usize;
        let upper = rank.ceil() as usize;
        if lower == upper || upper >= sorted.len() {
            sorted[lower]
        } else {
            let weight = rank - (lower as f64);
            sorted[lower] * (1.0 - weight) + sorted[upper] * weight
        }
    }
}

// ---------------------------------------------------------------------------
// Execution Trace Logger Core
// ---------------------------------------------------------------------------

/// Thread-safe execution trace logger writing structured JSONL to disk and in-memory buffer.
#[derive(Debug)]
pub struct ExecutionTraceLogger {
    log_path: PathBuf,
    writer: Mutex<Option<BufWriter<File>>>,
    memory_buffer: RwLock<Vec<TraceRecord>>,
    buffer_capacity: usize,
}

impl ExecutionTraceLogger {
    /// Creates a new trace logger writing to the specified path or default `~/.fusion/traces/execution_traces.jsonl`.
    pub fn new(log_path: Option<PathBuf>) -> Self {
        let path = log_path.unwrap_or_else(execution_traces_path);
        Self {
            log_path: path,
            writer: Mutex::new(None),
            memory_buffer: RwLock::new(Vec::new()),
            buffer_capacity: 10_000,
        }
    }

    /// Sets in-memory buffer capacity limit.
    pub fn with_buffer_capacity(mut self, capacity: usize) -> Self {
        self.buffer_capacity = capacity;
        self
    }

    /// Path to the JSONL log file.
    pub fn log_path(&self) -> &Path {
        &self.log_path
    }

    fn ensure_writer(&self) -> anyhow::Result<()> {
        let mut guard = self
            .writer
            .lock()
            .map_err(|_| anyhow::anyhow!("Writer lock poisoned"))?;
        if guard.is_none() {
            if let Some(parent) = self.log_path.parent() {
                if !parent.exists() {
                    fs::create_dir_all(parent)?;
                }
            }
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.log_path)?;
            *guard = Some(BufWriter::new(file));
        }
        Ok(())
    }

    /// Records a completed trace record to JSONL disk and in-memory buffer.
    pub fn record(&self, record: TraceRecord) -> anyhow::Result<()> {
        let json_line = record.to_jsonl_line()?;

        // Write to file
        self.ensure_writer()?;
        if let Ok(mut guard) = self.writer.lock() {
            if let Some(w) = guard.as_mut() {
                w.write_all(json_line.as_bytes())?;
                w.write_all(b"\n")?;
                let _ = w.flush();
            }
        }

        // Store in memory ring buffer
        if let Ok(mut mem) = self.memory_buffer.write() {
            if mem.len() >= self.buffer_capacity {
                let drain_count = mem.len() - self.buffer_capacity + 1;
                mem.drain(0..drain_count);
            }
            mem.push(record);
        }

        Ok(())
    }

    /// Flushes the active file writer to disk.
    pub fn flush(&self) -> anyhow::Result<()> {
        if let Ok(mut guard) = self.writer.lock() {
            if let Some(w) = guard.as_mut() {
                w.flush()?;
            }
        }
        Ok(())
    }

    /// Starts an agent turn span and returns an RAII guard.
    pub fn start_turn(
        &self,
        session_id: &str,
        turn_index: usize,
        model: Option<&str>,
    ) -> SpanGuard<'_> {
        let trace_id = create_trace_id();
        let span_id = create_span_id();
        let name = format!("turn:{}", turn_index);

        let mut attributes = HashMap::new();
        if let Some(m) = model {
            attributes.insert(
                "model".to_string(),
                serde_json::Value::String(m.to_string()),
            );
        }

        SpanGuard::new(
            self,
            trace_id,
            span_id,
            None,
            Some(session_id.to_string()),
            Some(turn_index),
            TraceEventType::AgentTurn,
            name,
            attributes,
        )
    }

    /// Starts a tool invocation span and returns an RAII guard.
    pub fn start_tool(
        &self,
        session_id: &str,
        turn_index: Option<usize>,
        tool_name: &str,
        call_id: &str,
        arguments: &str,
        parent_span_id: Option<&str>,
    ) -> SpanGuard<'_> {
        let trace_id = create_trace_id();
        let span_id = create_span_id();
        let name = format!("tool:{}", tool_name);

        let mut guard = SpanGuard::new(
            self,
            trace_id,
            span_id,
            parent_span_id.map(|s| s.to_string()),
            Some(session_id.to_string()),
            turn_index,
            TraceEventType::ToolInvocation,
            name,
            HashMap::new(),
        );

        guard.record.tool_metadata = Some(TraceToolMetadata {
            tool_name: tool_name.to_string(),
            call_id: call_id.to_string(),
            arguments_summary: Some(arguments.chars().take(500).collect()),
            output_bytes: 0,
            is_truncated: false,
            is_error: false,
        });

        guard
    }

    /// Starts an LLM request span and returns an RAII guard.
    pub fn start_llm_request(
        &self,
        session_id: &str,
        model: &str,
        provider: Option<&str>,
        parent_span_id: Option<&str>,
    ) -> SpanGuard<'_> {
        let trace_id = create_trace_id();
        let span_id = create_span_id();
        let name = format!("llm:{}", model);

        let mut guard = SpanGuard::new(
            self,
            trace_id,
            span_id,
            parent_span_id.map(|s| s.to_string()),
            Some(session_id.to_string()),
            None,
            TraceEventType::LlmRequest,
            name,
            HashMap::new(),
        );

        guard.record.llm_metadata = Some(TraceLlmMetadata {
            model: model.to_string(),
            provider: provider.map(|p| p.to_string()),
            temperature: None,
            finish_reason: None,
            first_token_latency_ms: None,
        });

        guard
    }

    /// Starts a custom span with full parameter control.
    pub fn start_span(
        &self,
        name: &str,
        event_type: TraceEventType,
        session_id: Option<&str>,
        parent_span_id: Option<&str>,
    ) -> SpanGuard<'_> {
        let trace_id = create_trace_id();
        let span_id = create_span_id();

        SpanGuard::new(
            self,
            trace_id,
            span_id,
            parent_span_id.map(|s| s.to_string()),
            session_id.map(|s| s.to_string()),
            None,
            event_type,
            name.to_string(),
            HashMap::new(),
        )
    }

    /// Queries trace records from the active JSONL log file matching the given filter.
    pub fn query(&self, filter: &TraceQuery) -> anyhow::Result<Vec<TraceRecord>> {
        if self.log_path.exists() {
            TraceReader::filter_file(&self.log_path, filter)
        } else {
            // Query in-memory buffer fallback
            let mem = self
                .memory_buffer
                .read()
                .map_err(|_| anyhow::anyhow!("Memory buffer lock poisoned"))?;
            let mut results = Vec::new();
            let mut skipped = 0;
            for r in mem.iter() {
                if filter.matches(r) {
                    if let Some(offset) = filter.offset {
                        if skipped < offset {
                            skipped += 1;
                            continue;
                        }
                    }
                    results.push(r.clone());
                    if let Some(limit) = filter.limit {
                        if results.len() >= limit {
                            break;
                        }
                    }
                }
            }
            Ok(results)
        }
    }

    /// Computes aggregated analytics from query results.
    pub fn query_analytics(&self, filter: &TraceQuery) -> anyhow::Result<TraceAnalytics> {
        let records = self.query(filter)?;
        Ok(TraceReader::compute_analytics(&records))
    }

    /// Returns a copy of recent in-memory trace records.
    pub fn memory_records(&self) -> Vec<TraceRecord> {
        self.memory_buffer
            .read()
            .map(|m| m.clone())
            .unwrap_or_default()
    }

    /// Clears the in-memory trace buffer.
    pub fn clear_memory_buffer(&self) {
        if let Ok(mut m) = self.memory_buffer.write() {
            m.clear();
        }
    }

    /// Exports matched spans in standard OpenTelemetry (OTLP) JSON format.
    pub fn export_otlp_payload(
        &self,
        filter: Option<&TraceQuery>,
    ) -> anyhow::Result<OtelExportPayload> {
        let empty_filter = TraceQuery::default();
        let effective_filter = filter.unwrap_or(&empty_filter);
        let records = self.query(effective_filter)?;

        let mut spans = Vec::with_capacity(records.len());
        for r in records {
            spans.push(r.to_otel_span());
        }

        let scope_spans = vec![OtelScopeSpans {
            scope: OtelInstrumentationScope {
                name: "fusion.agent.trace".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            },
            spans,
        }];

        let resource_spans = vec![OtelResourceSpans {
            resource: OtelResource {
                attributes: vec![
                    OtelKeyValue {
                        key: "service.name".to_string(),
                        value: "fusion".into(),
                    },
                    OtelKeyValue {
                        key: "service.version".to_string(),
                        value: env!("CARGO_PKG_VERSION").into(),
                    },
                    OtelKeyValue {
                        key: "host.arch".to_string(),
                        value: std::env::consts::ARCH.into(),
                    },
                    OtelKeyValue {
                        key: "os.type".to_string(),
                        value: std::env::consts::OS.into(),
                    },
                ],
            },
            scope_spans,
        }];

        Ok(OtelExportPayload { resource_spans })
    }

    /// Exports matched spans as an OTLP JSON string.
    pub fn export_otlp_json_string(&self, filter: Option<&TraceQuery>) -> anyhow::Result<String> {
        let payload = self.export_otlp_payload(filter)?;
        Ok(serde_json::to_string_pretty(&payload)?)
    }
}

/// RAII span guard that measures duration and automatically logs completed trace records.
pub struct SpanGuard<'a> {
    logger: &'a ExecutionTraceLogger,
    pub record: TraceRecord,
    start_instant: Instant,
    is_finished: bool,
}

impl<'a> SpanGuard<'a> {
    fn new(
        logger: &'a ExecutionTraceLogger,
        trace_id: String,
        span_id: String,
        parent_span_id: Option<String>,
        session_id: Option<String>,
        turn_index: Option<usize>,
        event_type: TraceEventType,
        name: String,
        attributes: HashMap<String, serde_json::Value>,
    ) -> Self {
        let now_utc = Utc::now();
        let start_time = now_utc.to_rfc3339();
        let start_time_unix_nano = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);

        let record = TraceRecord {
            record_id: Uuid::new_v4(),
            trace_id,
            span_id,
            parent_span_id,
            session_id,
            turn_index,
            event_type,
            name,
            start_time,
            start_time_unix_nano,
            end_time: None,
            end_time_unix_nano: None,
            duration_ms: None,
            status: TraceStatus::InProgress,
            token_metrics: None,
            tool_metadata: None,
            llm_metadata: None,
            attributes,
            events: Vec::new(),
        };

        Self {
            logger,
            record,
            start_instant: Instant::now(),
            is_finished: false,
        }
    }

    /// Span unique identifier (16 hex chars).
    pub fn span_id(&self) -> &str {
        &self.record.span_id
    }

    /// Trace unique identifier (32 hex chars).
    pub fn trace_id(&self) -> &str {
        &self.record.trace_id
    }

    /// Sets explicit execution status on this span.
    pub fn set_status(&mut self, status: TraceStatus) {
        self.record.status = status;
    }

    /// Marks the span status as successful.
    pub fn set_ok(&mut self) {
        self.record.status = TraceStatus::Ok;
    }

    /// Marks the span status as an error with message.
    pub fn set_error(&mut self, message: impl Into<String>) {
        self.record.status = TraceStatus::Error {
            message: message.into(),
        };
    }

    /// Attaches token consumption metrics.
    pub fn set_token_metrics(&mut self, metrics: TraceTokenMetrics) {
        self.record.token_metrics = Some(metrics);
    }

    /// Attaches token stats converted from TokenStats.
    pub fn set_token_stats(&mut self, stats: TokenStats) {
        self.record.token_metrics = Some(TraceTokenMetrics::from(stats));
    }

    /// Adds or replaces a custom attribute.
    pub fn set_attribute(&mut self, key: impl Into<String>, value: impl Into<serde_json::Value>) {
        self.record.attributes.insert(key.into(), value.into());
    }

    /// Adds an in-span timestamped log event.
    pub fn add_event(
        &mut self,
        name: impl Into<String>,
        attributes: HashMap<String, serde_json::Value>,
    ) {
        let now_utc = Utc::now();
        let timestamp = now_utc.to_rfc3339();
        let timestamp_unix_nano = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);

        self.record.events.push(TraceLogEvent {
            name: name.into(),
            timestamp,
            timestamp_unix_nano,
            attributes,
        });
    }

    /// Sets tool output summary and automatically marks status.
    pub fn set_tool_output(&mut self, output: &str, is_error: bool) {
        if let Some(tool) = &mut self.record.tool_metadata {
            tool.output_bytes = output.len();
            tool.is_error = is_error;
        }
        if is_error {
            self.set_error(output.chars().take(200).collect::<String>());
        } else {
            self.set_ok();
        }
    }

    /// Manually finalizes this span and returns the completed record.
    pub fn finish(mut self) -> TraceRecord {
        self.finalize();
        self.record.clone()
    }

    fn finalize(&mut self) {
        if self.is_finished {
            return;
        }
        self.is_finished = true;

        let elapsed = self.start_instant.elapsed();
        let duration_ms = elapsed.as_secs_f64() * 1000.0;
        let now_utc = Utc::now();
        let end_time = now_utc.to_rfc3339();
        let end_time_unix_nano = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);

        self.record.duration_ms = Some(duration_ms);
        self.record.end_time = Some(end_time);
        self.record.end_time_unix_nano = Some(end_time_unix_nano);

        if self.record.status == TraceStatus::InProgress {
            self.record.status = TraceStatus::Ok;
        }

        let _ = self.logger.record(self.record.clone());
    }
}

impl<'a> Drop for SpanGuard<'a> {
    fn drop(&mut self) {
        self.finalize();
    }
}

/// Returns the default JSONL trace log path: `~/.fusion/traces/execution_traces.jsonl`.
pub fn execution_traces_path() -> PathBuf {
    traces_dir().join("execution_traces.jsonl")
}

/// Generates a standard OpenTelemetry 128-bit (32 hex char) trace identifier.
pub fn create_trace_id() -> String {
    format!("{:032x}", Uuid::new_v4().as_u128())
}

/// Generates a standard OpenTelemetry 64-bit (16 hex char) span identifier.
pub fn create_span_id() -> String {
    format!(
        "{:016x}",
        (Uuid::new_v4().as_u128() & 0xFFFF_FFFF_FFFF_FFFF) as u64
    )
}

/// Global shared execution trace logger instance.
static GLOBAL_LOGGER: LazyLock<ExecutionTraceLogger> =
    LazyLock::new(|| ExecutionTraceLogger::new(None));

/// Returns a reference to the global shared execution trace logger instance.
pub fn global_trace_logger() -> &'static ExecutionTraceLogger {
    &GLOBAL_LOGGER
}

// ---------------------------------------------------------------------------
// Unit Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::session::Session;
    use crate::provider::types::{Message, ToolCall};

    #[test]
    fn test_redaction_api_keys() {
        let redactor = TraceRedactor::new();
        let mut audit = RedactionAudit::default();

        let raw = "Using Anthropic sk-ant-api03-abcdef1234567890abcdef123456 and OpenAI sk-proj-1234567890abcdef1234567890.";
        let redacted = redactor.redact(raw, &mut audit);

        assert!(!redacted.contains("sk-ant-"));
        assert!(!redacted.contains("sk-proj-"));
        assert!(redacted.contains("[REDACTED_API_KEY]"));
        assert_eq!(audit.api_keys, 2);
        assert_eq!(audit.total_redactions, 2);
    }

    #[test]
    fn test_redaction_private_keys() {
        let redactor = TraceRedactor::new();
        let mut audit = RedactionAudit::default();

        let raw = "Config:\n-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEA0Y...\n-----END RSA PRIVATE KEY-----\nEnd of config.";
        let redacted = redactor.redact(raw, &mut audit);

        assert!(!redacted.contains("MIIEowIBAAKCAQEA0Y"));
        assert!(redacted.contains("[REDACTED_PRIVATE_KEY]"));
        assert_eq!(audit.private_keys, 1);
    }

    #[test]
    fn test_redaction_tokens_and_passwords() {
        let redactor = TraceRedactor::new();
        let mut audit = RedactionAudit::default();

        let raw = r#"{"username": "admin", "password": "supersecretpassword123", "token": "ghp_1234567890abcdef1234567890"}"#;
        let redacted = redactor.redact(raw, &mut audit);

        assert!(!redacted.contains("supersecretpassword123"));
        assert!(!redacted.contains("ghp_1234567890abcdef"));
        assert!(redacted.contains(r#""password": "[REDACTED_SECRET]""#));
        assert!(redacted.contains("[REDACTED_GITHUB_TOKEN]"));
        assert!(audit.passwords_and_secrets >= 1);
        assert!(audit.auth_tokens >= 1);
    }

    #[test]
    fn test_redaction_email_and_ip() {
        let redactor = TraceRedactor::new();
        let mut audit = RedactionAudit::default();

        let raw = "Contact alice@example.com at host 192.168.1.50 or loopback 127.0.0.1";
        let redacted = redactor.redact(raw, &mut audit);

        assert!(!redacted.contains("alice@example.com"));
        assert!(!redacted.contains("192.168.1.50"));
        assert!(redacted.contains("[REDACTED_EMAIL]"));
        assert!(redacted.contains("[REDACTED_IP]"));
        assert!(redacted.contains("127.0.0.1")); // Loopback preserved
        assert_eq!(audit.emails, 1);
        assert_eq!(audit.ip_addresses, 1);
    }

    #[test]
    fn test_redaction_home_and_user_paths() {
        let mut redactor = TraceRedactor::new();
        redactor.home_dir = Some("/Users/testuser".to_string());
        let mut audit = RedactionAudit::default();

        let raw = "File saved to /Users/testuser/project/file.rs and /home/bob/other.rs";
        let redacted = redactor.redact(raw, &mut audit);

        assert!(!redacted.contains("/Users/testuser"));
        assert!(!redacted.contains("/home/bob"));
        assert!(redacted.contains("~/project/file.rs"));
        assert!(redacted.contains("/home/[USER]/other.rs"));
        assert!(audit.user_paths >= 2);
    }

    #[test]
    fn test_redaction_exact_known_secrets() {
        let redactor = TraceRedactor::new().with_known_secrets(&["custom_super_secret_key_12345"]);
        let mut audit = RedactionAudit::default();

        let raw = "Debug: used secret custom_super_secret_key_12345 for auth";
        let redacted = redactor.redact(raw, &mut audit);

        assert!(!redacted.contains("custom_super_secret_key_12345"));
        assert!(redacted.contains("[REDACTED_API_KEY]"));
        assert_eq!(audit.api_keys, 1);
    }

    #[test]
    fn test_diagnostic_trace_generation_and_markdown() {
        let mut session = Session::new("deepseek-chat");
        session.set_title("Test Debug Session");
        session.add_user_message("Please search for main function in src/");

        let tool_call = ToolCall {
            id: "call_123".to_string(),
            name: "grep".to_string(),
            arguments: r#"{"pattern": "fn main", "path": "/Users/alice/repo"}"#.to_string(),
        };

        session.add_assistant_with_tools("I will run grep tool.", vec![tool_call]);
        session.add_tool_result(
            "call_123",
            "Found in /Users/alice/repo/src/main.rs:1: fn main() { ... }",
        );
        session.add_assistant_message("I found the main function.");

        session.record_usage(120, 45);

        let trace = generate_trace(&session, None, None);

        assert_eq!(trace.session_metadata.active_model, "deepseek-chat");
        assert_eq!(
            trace.session_metadata.title.as_deref(),
            Some("Test Debug Session")
        );
        assert_eq!(trace.tool_executions.len(), 1);
        assert_eq!(trace.tool_executions[0].tool_name, "grep");
        assert_eq!(trace.session_metadata.token_stats.prompt_tokens, 120);
        assert_eq!(trace.session_metadata.token_stats.completion_tokens, 45);

        let markdown = trace.to_markdown();
        assert!(markdown.contains("# Fusion Diagnostic Trace"));
        assert!(markdown.contains("## 1. Executive Summary"));
        assert!(markdown.contains("## 2. Privacy & Redaction Audit"));
        assert!(markdown.contains("## 3. System Environment"));
        assert!(markdown.contains("## 5. Tool Execution Logs"));
        assert!(markdown.contains("Tool #1: `grep`"));

        // Verify that /Users/alice is redacted
        assert!(!markdown.contains("/Users/alice"));
    }

    #[test]
    fn test_save_trace_file() {
        let session = Session::new("claude-3-5-sonnet");
        let temp_dir = std::env::temp_dir().join(format!("fusion_test_trace_{}", Uuid::new_v4()));
        let dest = temp_dir.join("trace.md");

        let (saved_path, _audit) = save_trace_file(&session, None, Some(&dest), None).unwrap();

        assert_eq!(saved_path, dest);
        assert!(saved_path.exists());
        let content = fs::read_to_string(&saved_path).unwrap();
        assert!(content.contains("# Fusion Diagnostic Trace"));

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn test_custom_pattern_redaction() {
        let redactor = TraceRedactor::new()
            .with_custom_pattern(r"INTERNAL_[A-Z0-9]+")
            .unwrap();
        let mut audit = RedactionAudit::default();

        let raw = "Project contains INTERNAL_PROJECT_ID_9999 secret code.";
        let redacted = redactor.redact(raw, &mut audit);

        assert!(!redacted.contains("INTERNAL_PROJECT_ID_9999"));
        assert!(redacted.contains("[REDACTED_CUSTOM]"));
        assert_eq!(audit.custom, 1);
    }

    #[test]
    fn test_empty_session_trace() {
        let session = Session::new("gpt-4o");
        let trace = generate_trace(&session, None, None);

        assert_eq!(trace.session_metadata.active_model, "gpt-4o");
        assert_eq!(trace.session_metadata.total_messages, 0);
        assert!(trace.tool_executions.is_empty());
        assert!(trace.redaction_audit.is_clean());

        let md = trace.to_markdown();
        assert!(md.contains("No tool executions recorded"));
        assert!(md.contains("No sensitive data detected (clean)"));
    }

    #[test]
    fn test_trace_record_jsonl_roundtrip() {
        let trace_id = create_trace_id();
        let span_id = create_span_id();
        let mut attributes = HashMap::new();
        attributes.insert("user.id".to_string(), serde_json::json!("alice_123"));
        attributes.insert("retry.count".to_string(), serde_json::json!(3));

        let record = TraceRecord {
            record_id: Uuid::new_v4(),
            trace_id: trace_id.clone(),
            span_id: span_id.clone(),
            parent_span_id: None,
            session_id: Some("session-abc-456".to_string()),
            turn_index: Some(1),
            event_type: TraceEventType::ToolInvocation,
            name: "tool:grep".to_string(),
            start_time: "2026-09-02T10:00:00Z".to_string(),
            start_time_unix_nano: 1788343200000000000,
            end_time: Some("2026-09-02T10:00:00.125Z".to_string()),
            end_time_unix_nano: Some(1788343200125000000),
            duration_ms: Some(125.0),
            status: TraceStatus::Ok,
            token_metrics: Some(TraceTokenMetrics {
                prompt_tokens: 150,
                completion_tokens: 50,
                total_tokens: 200,
                cache_read_tokens: 20,
                cache_write_tokens: 0,
            }),
            tool_metadata: Some(TraceToolMetadata {
                tool_name: "grep".to_string(),
                call_id: "call_999".to_string(),
                arguments_summary: Some(r#"{"pattern":"main"}"#.to_string()),
                output_bytes: 4096,
                is_truncated: false,
                is_error: false,
            }),
            llm_metadata: None,
            attributes,
            events: vec![TraceLogEvent {
                name: "search_started".to_string(),
                timestamp: "2026-09-02T10:00:00.010Z".to_string(),
                timestamp_unix_nano: 1788343200010000000,
                attributes: HashMap::new(),
            }],
        };

        let jsonl_line = record.to_jsonl_line().expect("failed to serialize");
        assert!(!jsonl_line.contains('\n'));

        let deserialized =
            TraceRecord::from_jsonl_line(&jsonl_line).expect("failed to deserialize");
        assert_eq!(deserialized.record_id, record.record_id);
        assert_eq!(deserialized.trace_id, trace_id);
        assert_eq!(deserialized.span_id, span_id);
        assert_eq!(deserialized.event_type, TraceEventType::ToolInvocation);
        assert_eq!(deserialized.name, "tool:grep");
        assert_eq!(deserialized.duration_ms, Some(125.0));
        assert_eq!(deserialized.status, TraceStatus::Ok);
        assert_eq!(deserialized.token_metrics.unwrap().total_tokens, 200);
        assert_eq!(
            deserialized.tool_metadata.as_ref().unwrap().tool_name,
            "grep"
        );
        assert_eq!(deserialized.events.len(), 1);
    }

    #[test]
    fn test_span_guard_timing_and_lifecycle() {
        let temp_dir = std::env::temp_dir().join(format!("fusion_test_sg_{}", Uuid::new_v4()));
        let log_file = temp_dir.join("traces.jsonl");
        let logger = ExecutionTraceLogger::new(Some(log_file.clone()));

        {
            let mut guard = logger.start_turn("sess-1", 1, Some("deepseek-chat"));
            assert_eq!(guard.record.session_id.as_deref(), Some("sess-1"));
            assert_eq!(guard.record.turn_index, Some(1));
            guard.set_token_metrics(TraceTokenMetrics {
                prompt_tokens: 100,
                completion_tokens: 30,
                total_tokens: 130,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
            });
            guard.set_attribute("experiment", "v2");
            guard.add_event("llm_dispatched", HashMap::new());
            // guard drops here and automatically finalizes & flushes to logger
        }

        let records = logger.memory_records();
        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(record.name, "turn:1");
        assert_eq!(record.event_type, TraceEventType::AgentTurn);
        assert!(record.duration_ms.is_some());
        assert!(record.duration_ms.unwrap() >= 0.0);
        assert_eq!(record.status, TraceStatus::Ok);
        assert_eq!(record.token_metrics.unwrap().total_tokens, 130);
        assert_eq!(record.events.len(), 1);

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn test_filter_by_session_id() {
        let temp_dir = std::env::temp_dir().join(format!("fusion_test_sid_{}", Uuid::new_v4()));
        let log_file = temp_dir.join("traces.jsonl");
        let logger = ExecutionTraceLogger::new(Some(log_file.clone()));

        for i in 1..=5 {
            let sess = if i % 2 == 0 { "session-A" } else { "session-B" };
            let guard = logger.start_turn(sess, i, None);
            guard.finish();
        }

        let query_a = TraceQuery::new().with_session_id("session-A");
        let results_a = logger.query(&query_a).expect("query failed");
        assert_eq!(results_a.len(), 2);
        for r in results_a {
            assert_eq!(r.session_id.as_deref(), Some("session-A"));
        }

        let query_b = TraceQuery::new().with_session_id("session-B");
        let results_b = logger.query(&query_b).expect("query failed");
        assert_eq!(results_b.len(), 3);

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn test_filter_by_tool_name() {
        let temp_dir = std::env::temp_dir().join(format!("fusion_test_tool_{}", Uuid::new_v4()));
        let log_file = temp_dir.join("traces.jsonl");
        let logger = ExecutionTraceLogger::new(Some(log_file.clone()));

        let g1 = logger.start_tool("s1", Some(1), "grep", "c1", r#"{"pattern":"foo"}"#, None);
        g1.finish();

        let g2 = logger.start_tool("s1", Some(1), "file_read", "c2", r#"{"path":"a.rs"}"#, None);
        g2.finish();

        let g3 = logger.start_tool("s1", Some(2), "grep", "c3", r#"{"pattern":"bar"}"#, None);
        g3.finish();

        let query_grep = TraceQuery::new().with_tool_name("grep");
        let results_grep = logger.query(&query_grep).expect("query failed");
        assert_eq!(results_grep.len(), 2);

        let query_read = TraceQuery::new().with_tool_name("file_read");
        let results_read = logger.query(&query_read).expect("query failed");
        assert_eq!(results_read.len(), 1);

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn test_filter_by_error_status() {
        let temp_dir = std::env::temp_dir().join(format!("fusion_test_err_{}", Uuid::new_v4()));
        let log_file = temp_dir.join("traces.jsonl");
        let logger = ExecutionTraceLogger::new(Some(log_file.clone()));

        let mut g1 = logger.start_tool("s1", Some(1), "bash", "c1", "cargo test", None);
        g1.set_tool_output("Tests passed successfully", false);
        g1.finish();

        let mut g2 = logger.start_tool("s1", Some(1), "bash", "c2", "cargo check", None);
        g2.set_tool_output("Error: syntax error in main.rs", true);
        g2.finish();

        let err_query = TraceQuery::new().errors_only();
        let errs = logger.query(&err_query).expect("query failed");
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].status.is_error(), true);
        assert!(errs[0].tool_metadata.as_ref().unwrap().is_error);

        let ok_query = TraceQuery::new().success_only();
        let oks = logger.query(&ok_query).expect("query failed");
        assert_eq!(oks.len(), 1);
        assert_eq!(oks[0].status.is_ok(), true);

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn test_analytics_and_percentiles() {
        let records = vec![
            TraceRecord {
                record_id: Uuid::new_v4(),
                trace_id: create_trace_id(),
                span_id: create_span_id(),
                parent_span_id: None,
                session_id: Some("s1".to_string()),
                turn_index: Some(1),
                event_type: TraceEventType::ToolInvocation,
                name: "tool:grep".to_string(),
                start_time: "2026-09-02T10:00:00Z".to_string(),
                start_time_unix_nano: 100,
                end_time: None,
                end_time_unix_nano: None,
                duration_ms: Some(10.0),
                status: TraceStatus::Ok,
                token_metrics: Some(TraceTokenMetrics {
                    prompt_tokens: 100,
                    completion_tokens: 50,
                    total_tokens: 150,
                    cache_read_tokens: 0,
                    cache_write_tokens: 0,
                }),
                tool_metadata: Some(TraceToolMetadata {
                    tool_name: "grep".to_string(),
                    call_id: "c1".to_string(),
                    arguments_summary: None,
                    output_bytes: 500,
                    is_truncated: false,
                    is_error: false,
                }),
                llm_metadata: None,
                attributes: HashMap::new(),
                events: Vec::new(),
            },
            TraceRecord {
                record_id: Uuid::new_v4(),
                trace_id: create_trace_id(),
                span_id: create_span_id(),
                parent_span_id: None,
                session_id: Some("s1".to_string()),
                turn_index: Some(1),
                event_type: TraceEventType::ToolInvocation,
                name: "tool:grep".to_string(),
                start_time: "2026-09-02T10:00:01Z".to_string(),
                start_time_unix_nano: 200,
                end_time: None,
                end_time_unix_nano: None,
                duration_ms: Some(30.0),
                status: TraceStatus::Error {
                    message: "timeout".to_string(),
                },
                token_metrics: None,
                tool_metadata: Some(TraceToolMetadata {
                    tool_name: "grep".to_string(),
                    call_id: "c2".to_string(),
                    arguments_summary: None,
                    output_bytes: 0,
                    is_truncated: false,
                    is_error: true,
                }),
                llm_metadata: None,
                attributes: HashMap::new(),
                events: Vec::new(),
            },
            TraceRecord {
                record_id: Uuid::new_v4(),
                trace_id: create_trace_id(),
                span_id: create_span_id(),
                parent_span_id: None,
                session_id: Some("s1".to_string()),
                turn_index: Some(2),
                event_type: TraceEventType::AgentTurn,
                name: "turn:2".to_string(),
                start_time: "2026-09-02T10:00:02Z".to_string(),
                start_time_unix_nano: 300,
                end_time: None,
                end_time_unix_nano: None,
                duration_ms: Some(60.0),
                status: TraceStatus::Ok,
                token_metrics: Some(TraceTokenMetrics {
                    prompt_tokens: 200,
                    completion_tokens: 100,
                    total_tokens: 300,
                    cache_read_tokens: 0,
                    cache_write_tokens: 0,
                }),
                tool_metadata: None,
                llm_metadata: None,
                attributes: HashMap::new(),
                events: Vec::new(),
            },
        ];

        let analytics = TraceReader::compute_analytics(&records);
        assert_eq!(analytics.total_records, 3);
        assert_eq!(analytics.total_turns, 1);
        assert_eq!(analytics.total_tool_calls, 2);
        assert_eq!(analytics.total_errors, 1);
        assert_eq!(analytics.total_duration_ms, 100.0);
        assert_eq!(analytics.min_duration_ms, 10.0);
        assert_eq!(analytics.max_duration_ms, 60.0);
        assert_eq!(analytics.p50_duration_ms, 30.0);
        assert_eq!(analytics.total_tokens.total_tokens, 450);

        let grep_stats = analytics.tool_breakdown.get("grep").unwrap();
        assert_eq!(grep_stats.call_count, 2);
        assert_eq!(grep_stats.error_count, 1);
        assert_eq!(grep_stats.success_rate, 0.5);
        assert_eq!(grep_stats.total_output_bytes, 500);
    }

    #[test]
    fn test_opentelemetry_span_conversion() {
        let trace_id = create_trace_id();
        let span_id = create_span_id();

        let record = TraceRecord {
            record_id: Uuid::new_v4(),
            trace_id: trace_id.clone(),
            span_id: span_id.clone(),
            parent_span_id: Some("0011223344556677".to_string()),
            session_id: Some("sess-test".to_string()),
            turn_index: Some(3),
            event_type: TraceEventType::ToolInvocation,
            name: "tool:bash".to_string(),
            start_time: "2026-09-02T10:00:00Z".to_string(),
            start_time_unix_nano: 1000000,
            end_time: Some("2026-09-02T10:00:01Z".to_string()),
            end_time_unix_nano: Some(2000000),
            duration_ms: Some(1000.0),
            status: TraceStatus::Ok,
            token_metrics: Some(TraceTokenMetrics {
                prompt_tokens: 40,
                completion_tokens: 10,
                total_tokens: 50,
                cache_read_tokens: 5,
                cache_write_tokens: 0,
            }),
            tool_metadata: Some(TraceToolMetadata {
                tool_name: "bash".to_string(),
                call_id: "c_bash".to_string(),
                arguments_summary: Some("echo hello".to_string()),
                output_bytes: 12,
                is_truncated: false,
                is_error: false,
            }),
            llm_metadata: None,
            attributes: HashMap::new(),
            events: Vec::new(),
        };

        let otel_span = record.to_otel_span();
        assert_eq!(otel_span.trace_id, trace_id);
        assert_eq!(otel_span.span_id, span_id);
        assert_eq!(
            otel_span.parent_span_id.as_deref(),
            Some("0011223344556677")
        );
        assert_eq!(otel_span.kind, OtelSpanKind::Client as i32);
        assert_eq!(otel_span.status.code, OtelStatusCode::Ok as i32);

        let has_session_attr = otel_span.attributes.iter().any(|kv| kv.key == "session.id");
        assert!(has_session_attr);

        let has_tool_attr = otel_span.attributes.iter().any(|kv| kv.key == "tool.name");
        assert!(has_tool_attr);
    }

    #[test]
    fn test_opentelemetry_export_payload() {
        let temp_dir = std::env::temp_dir().join(format!("fusion_test_otlp_{}", Uuid::new_v4()));
        let log_file = temp_dir.join("traces.jsonl");
        let logger = ExecutionTraceLogger::new(Some(log_file.clone()));

        let g = logger.start_turn("sess-otlp", 1, Some("claude-3-5-sonnet"));
        g.finish();

        let payload = logger
            .export_otlp_payload(None)
            .expect("export payload failed");
        assert_eq!(payload.resource_spans.len(), 1);
        assert_eq!(payload.resource_spans[0].scope_spans[0].spans.len(), 1);

        let json_str = logger
            .export_otlp_json_string(None)
            .expect("json export failed");
        assert!(json_str.contains("resource_spans"));
        assert!(json_str.contains("fusion.agent.trace"));

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn test_trace_reader_corrupt_line_handling() {
        let data = "{\"name\":\"valid_jsonl\"}\nNOT_VALID_JSON\n\n{\"record_id\":\"00000000-0000-0000-0000-000000000001\",\"trace_id\":\"t1\",\"span_id\":\"s1\",\"event_type\":\"agent_turn\",\"name\":\"turn:1\",\"start_time\":\"2026-09-02T10:00:00Z\",\"start_time_unix_nano\":100,\"status\":{\"status\":\"ok\"}}\n";
        let cursor = std::io::Cursor::new(data);
        let records = TraceReader::read_reader(cursor).expect("read failed");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].name, "turn:1");
    }
}
