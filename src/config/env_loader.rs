//! Robust, secure `.env` and configuration loader for Fusion.
//!
//! Features:
//! - Multi-tier hierarchy: CLI flags > process environment > local `.env` > project config > global config (`~/.config/fusion/config.json`).
//! - Cascading `.env` support (`.env` -> `.env.local` -> `.env.<mode>` -> `.env.<mode>.local`).
//! - Variable expansion with POSIX parameter substitution (`${VAR}`, `$VAR`, `${VAR:-default}`, `${VAR:=default}`, `${VAR:+alternate}`, `${VAR:?error}`).
//! - Cycle detection and recursion depth limits to prevent stack overflow or infinite loops.
//! - Single-quoted, double-quoted, unquoted, and multiline values.
//! - Escape sequence unescaping in double quotes (`\n`, `\r`, `\t`, `\\`, `\"`, `\$`, etc.).
//! - Automatic secret detection (by key name patterns and value heuristics/prefixes like `sk-...`, `sk-ant-...`, `xai-...`).
//! - Automatic secret masking with customizable styles (`Partial`, `Full`, `TypeOnly`, `Hash`).
//! - Safe display and debug formatting ensuring secrets are never leaked in logs or error traces.
//! - In-memory isolation with optional safe application to `std::env`.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::LazyLock;
use thiserror::Error;

/// Maximum recursion depth allowed during variable expansion.
const MAX_EXPANSION_DEPTH: usize = 32;

/// Known exact sensitive environment variable names (matched case-insensitively).
static KNOWN_SECRET_KEYS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    let mut s = HashSet::new();
    // AI Providers & Models
    s.insert("OPENAI_API_KEY");
    s.insert("OPENAI_ORG_ID");
    s.insert("OPENAI_PROJECT_ID");
    s.insert("ANTHROPIC_API_KEY");
    s.insert("CLAUDE_API_KEY");
    s.insert("DEEPSEEK_API_KEY");
    s.insert("GEMINI_API_KEY");
    s.insert("GOOGLE_API_KEY");
    s.insert("XAI_API_KEY");
    s.insert("OPENROUTER_API_KEY");
    s.insert("MISTRAL_API_KEY");
    s.insert("GROQ_API_KEY");
    s.insert("COHERE_API_KEY");
    s.insert("COHERE_TOKEN");
    s.insert("TOGETHER_API_KEY");
    s.insert("TOGETHERAI_API_KEY");
    s.insert("FIREWORKS_API_KEY");
    s.insert("PERPLEXITY_API_KEY");
    s.insert("PPLX_API_KEY");
    s.insert("VOYAGE_API_KEY");
    s.insert("AI21_API_KEY");
    s.insert("REPLICATE_API_TOKEN");
    s.insert("HF_TOKEN");
    s.insert("HUGGING_FACE_HUB_TOKEN");
    s.insert("HUGGINGFACE_TOKEN");
    s.insert("OLLAMA_API_KEY");

    // Cloud Providers & Infrastructure
    s.insert("AWS_ACCESS_KEY_ID");
    s.insert("AWS_SECRET_ACCESS_KEY");
    s.insert("AWS_SESSION_TOKEN");
    s.insert("AWS_SECURITY_TOKEN");
    s.insert("AZURE_OPENAI_API_KEY");
    s.insert("AZURE_API_KEY");
    s.insert("AZURE_CLIENT_SECRET");
    s.insert("CLOUDFLARE_API_TOKEN");
    s.insert("CLOUDFLARE_API_KEY");
    s.insert("CF_API_TOKEN");
    s.insert("CF_API_KEY");
    s.insert("VERCEL_TOKEN");
    s.insert("NETLIFY_AUTH_TOKEN");
    s.insert("FLY_API_TOKEN");
    s.insert("HEROKU_API_KEY");
    s.insert("DIGITALOCEAN_ACCESS_TOKEN");
    s.insert("DO_TOKEN");

    // Version Control & Repositories
    s.insert("GITHUB_TOKEN");
    s.insert("GH_TOKEN");
    s.insert("GITHUB_PAT");
    s.insert("GITLAB_TOKEN");
    s.insert("BITBUCKET_TOKEN");
    s.insert("GIT_PASSWORD");

    // Databases & Cache
    s.insert("DATABASE_URL");
    s.insert("DATABASE_PASSWORD");
    s.insert("DB_PASSWORD");
    s.insert("POSTGRES_PASSWORD");
    s.insert("PGPASSWORD");
    s.insert("MYSQL_PWD");
    s.insert("MYSQL_ROOT_PASSWORD");
    s.insert("REDIS_PASSWORD");
    s.insert("REDIS_AUTH");
    s.insert("MONGO_URI");
    s.insert("MONGODB_URI");
    s.insert("MONGO_PASSWORD");

    // Messaging & Integrations
    s.insert("SLACK_BOT_TOKEN");
    s.insert("SLACK_TOKEN");
    s.insert("SLACK_SIGNING_SECRET");
    s.insert("DISCORD_TOKEN");
    s.insert("DISCORD_BOT_TOKEN");
    s.insert("TELEGRAM_BOT_TOKEN");
    s.insert("TELEGRAM_TOKEN");

    // Payment & Communication APIs
    s.insert("STRIPE_SECRET_KEY");
    s.insert("STRIPE_API_KEY");
    s.insert("STRIPE_PUBLISHABLE_KEY");
    s.insert("SENDGRID_API_KEY");
    s.insert("TWILIO_AUTH_TOKEN");
    s.insert("MAILGUN_API_KEY");

    // Secrets Managers & Keyrings
    s.insert("VAULT_TOKEN");
    s.insert("BW_SESSION");
    s.insert("OP_SERVICE_ACCOUNT_TOKEN");
    s.insert("OP_SESSION");
    s.insert("ONEPASSWORD_TOKEN");
    s.insert("INFISICAL_TOKEN");

    // Package Registries
    s.insert("NPM_TOKEN");
    s.insert("PYPI_API_TOKEN");
    s.insert("CARGO_REGISTRY_TOKEN");
    s.insert("DOCKER_PASSWORD");

    // Generic Tokens & Secrets
    s.insert("API_KEY");
    s.insert("APIKEY");
    s.insert("SECRET_KEY");
    s.insert("SECRET");
    s.insert("ACCESS_KEY");
    s.insert("ACCESS_TOKEN");
    s.insert("AUTH_TOKEN");
    s.insert("BEARER_TOKEN");
    s.insert("PRIVATE_KEY");
    s.insert("ENCRYPTION_KEY");
    s.insert("MASTER_KEY");
    s.insert("SESSION_SECRET");
    s.insert("SIGNING_SECRET");
    s.insert("JWT_SECRET");
    s.insert("SSH_PRIVATE_KEY");
    s.insert("SSH_KEY");

    s
});

/// Substring patterns that indicate a sensitive environment variable name.
const SENSITIVE_KEY_PATTERNS: &[&str] = &[
    "SECRET",
    "TOKEN",
    "PASSWORD",
    "PASSWD",
    "APIKEY",
    "API_KEY",
    "AUTH",
    "PRIVATE_KEY",
    "PRIV_KEY",
    "CREDENTIAL",
    "SIGNING_KEY",
    "BEARER",
    "PASSPHRASE",
    "SESSION_KEY",
    "ENCRYPTION_KEY",
    "WEBHOOK_SECRET",
    "JWT",
    "SALT",
];

/// Known secret value prefixes indicating API keys or auth tokens.
const SENSITIVE_VALUE_PREFIXES: &[&str] = &[
    "sk-",         // OpenAI / DeepSeek / Anthropic
    "sk-ant-",     // Anthropic Claude
    "sk-proj-",    // OpenAI Project key
    "sk-or-",      // OpenRouter
    "xai-",        // xAI Grok
    "ghp_",        // GitHub Personal Access Token
    "gho_",        // GitHub OAuth Access Token
    "ghu_",        // GitHub User-to-Server Token
    "ghs_",        // GitHub Server-to-Server Token
    "ghr_",        // GitHub Refresh Token
    "glpat-",      // GitLab PAT
    "xoxb-",       // Slack Bot Token
    "xoxp-",       // Slack User Token
    "xapp-",       // Slack App Token
    "AIza",        // Google API Key
    "AKIA",        // AWS Access Key ID
    "ASIA",        // AWS Temporary Access Key ID
    "eyJh",        // JWT Token Header (base64)
    "Bearer ",     // Bearer Token
    "bearer ",     // Bearer Token (lower)
    "-----BEGIN ", // PEM Private Key / Certificate
    "npm_",        // npm access token
    "pypi-",       // PyPI API token
];

/// Precedence levels in the multi-tier configuration hierarchy.
/// Higher numeric value indicates higher priority (overrides lower tiers).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HierarchyTier {
    /// Built-in or fallback default value (Precedence 0)
    DefaultValue = 0,
    /// Global user configuration file (`~/.config/fusion/config.json` or `~/.fusion/config.json`) (Precedence 1)
    GlobalConfig = 1,
    /// Project-level configuration file (`./.fusion/config.json` or `./fusion.json`) (Precedence 2)
    ProjectConfig = 2,
    /// Local `.env` files (`.env`, `.env.local`, `.env.<mode>`, `.env.<mode>.local`) (Precedence 3)
    LocalDotEnv = 3,
    /// Host process environment variables (`std::env`) (Precedence 4)
    ProcessEnv = 4,
    /// Command-line argument flags and programmatic overrides (Precedence 5 - Highest)
    CliFlag = 5,
}

impl fmt::Display for HierarchyTier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DefaultValue => write!(f, "default"),
            Self::GlobalConfig => write!(f, "global-config"),
            Self::ProjectConfig => write!(f, "project-config"),
            Self::LocalDotEnv => write!(f, "local-dotenv"),
            Self::ProcessEnv => write!(f, "process-env"),
            Self::CliFlag => write!(f, "cli-flag"),
        }
    }
}

/// Errors that may occur during environment loading and parsing.
#[derive(Debug, Error)]
pub enum EnvError {
    /// File system I/O error.
    #[error("I/O error reading environment file '{path}': {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// Syntax parse error.
    #[error("Parse error at line {line}: {message}")]
    Parse { line: usize, message: String },

    /// Unclosed quote in variable value.
    #[error("Unclosed quote '{quote_char}' starting at line {line}")]
    UnclosedQuote { line: usize, quote_char: char },

    /// Cyclic variable dependency detected during expansion.
    #[error("Cyclic variable dependency detected for '{variable}': {}", path.join(" -> "))]
    CyclicVariable {
        variable: String,
        path: Vec<String>,
    },

    /// Undefined variable during `${VAR:?error}` evaluation.
    #[error("Required variable '{variable}' is undefined or empty: {message}")]
    UndefinedVariable {
        variable: String,
        message: String,
        line: Option<usize>,
    },

    /// Invalid variable identifier key.
    #[error("Invalid environment variable key '{key}' at line {line}")]
    InvalidKey { line: usize, key: String },

    /// Maximum expansion depth exceeded.
    #[error("Maximum variable expansion depth ({max_depth}) exceeded while resolving '{variable}'")]
    DepthLimitExceeded {
        variable: String,
        max_depth: usize,
    },
}

/// Masking style for sensitive environment variable values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MaskStyle {
    /// Partial masking preserving leading/trailing chars for debugging (e.g. `sk-ant-***...1234`).
    #[default]
    Partial,
    /// Complete masking replacing all characters with asterisks (e.g. `********`).
    Full,
    /// Masking displaying only the type and length (e.g. `[SECRET: length 48]`).
    TypeOnly,
    /// Deterministic short hash for safe correlation without revealing value (e.g. `[SECRET: #a1b2c3d4]`).
    Hash,
}

/// The origin of an environment variable in the multi-tier hierarchy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvSource {
    /// Command-line argument / flag override (Highest precedence).
    Cli,
    /// Host process environment (`std::env`).
    Process,
    /// Local `.env` file (`.env`, `.env.local`, `.env.<mode>`, etc.).
    LocalEnv(PathBuf),
    /// Project-level configuration file (`./.fusion/config.json` or `./fusion.json`).
    ProjectConfig(PathBuf),
    /// Global user configuration file (`~/.config/fusion/config.json` or `~/.fusion/config.json`).
    GlobalConfig(PathBuf),
    /// Generic file on disk.
    File(PathBuf),
    /// Parsed from an in-memory string.
    Inline,
    /// Evaluated from a fallback or default value expression.
    DefaultValue,
    /// Cascaded across multiple files.
    Cascade,
}

impl EnvSource {
    /// Returns the hierarchy precedence tier associated with this source.
    pub fn tier(&self) -> HierarchyTier {
        match self {
            Self::Cli => HierarchyTier::CliFlag,
            Self::Process => HierarchyTier::ProcessEnv,
            Self::LocalEnv(_) | Self::File(_) | Self::Inline | Self::Cascade => {
                HierarchyTier::LocalDotEnv
            }
            Self::ProjectConfig(_) => HierarchyTier::ProjectConfig,
            Self::GlobalConfig(_) => HierarchyTier::GlobalConfig,
            Self::DefaultValue => HierarchyTier::DefaultValue,
        }
    }
}

impl fmt::Display for EnvSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cli => write!(f, "cli"),
            Self::Process => write!(f, "process"),
            Self::LocalEnv(p) => write!(f, "local:{}", p.display()),
            Self::ProjectConfig(p) => write!(f, "project:{}", p.display()),
            Self::GlobalConfig(p) => write!(f, "global:{}", p.display()),
            Self::File(p) => write!(f, "file:{}", p.display()),
            Self::Inline => write!(f, "inline"),
            Self::DefaultValue => write!(f, "default"),
            Self::Cascade => write!(f, "cascade"),
        }
    }
}

/// A parsed environment variable entry with metadata, hierarchy tier, and secret detection.
#[derive(Clone, PartialEq, Eq)]
pub struct EnvVariable {
    /// The variable name / key.
    pub key: String,
    /// The resolved variable value.
    pub value: String,
    /// The source origin of this variable.
    pub source: EnvSource,
    /// Whether this variable is detected as sensitive/secret.
    pub is_secret: bool,
    /// Line number where this variable was defined (if from a file/string).
    pub line_number: Option<usize>,
}

impl EnvVariable {
    /// Create a new `EnvVariable` with automatic secret detection.
    pub fn new(key: impl Into<String>, value: impl Into<String>, source: EnvSource) -> Self {
        let key = key.into();
        let value = value.into();
        let is_secret = is_secret(&key, &value);
        Self {
            key,
            value,
            source,
            is_secret,
            line_number: None,
        }
    }

    /// Set line number where the variable was defined.
    pub fn with_line(mut self, line: usize) -> Self {
        self.line_number = Some(line);
        self
    }

    /// Mark explicitly as secret or non-secret.
    pub fn with_secret_flag(mut self, is_secret: bool) -> Self {
        self.is_secret = is_secret;
        self
    }

    /// Returns the hierarchy tier of this variable.
    pub fn tier(&self) -> HierarchyTier {
        self.source.tier()
    }

    /// Return the value masked according to the specified masking style.
    pub fn masked_value(&self, style: MaskStyle) -> String {
        if self.is_secret {
            mask_secret_value(&self.key, &self.value, style)
        } else {
            self.value.clone()
        }
    }
}

impl fmt::Debug for EnvVariable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EnvVariable")
            .field("key", &self.key)
            .field("value", &self.masked_value(MaskStyle::Partial))
            .field("source", &self.source)
            .field("tier", &self.tier())
            .field("is_secret", &self.is_secret)
            .field("line_number", &self.line_number)
            .finish()
    }
}

impl fmt::Display for EnvVariable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}={}",
            self.key,
            self.masked_value(MaskStyle::Partial)
        )
    }
}

/// Checks whether an environment variable key indicates sensitive data.
pub fn is_secret_key(key: &str) -> bool {
    let upper = key.to_ascii_uppercase();
    if KNOWN_SECRET_KEYS.contains(upper.as_str()) {
        return true;
    }
    for pat in SENSITIVE_KEY_PATTERNS {
        if upper.contains(pat) {
            return true;
        }
    }
    false
}

/// Checks whether an environment variable value indicates sensitive data.
pub fn is_secret_value(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return false;
    }
    for prefix in SENSITIVE_VALUE_PREFIXES {
        if trimmed.starts_with(prefix) {
            return true;
        }
    }
    // Check for PEM private key header
    if trimmed.contains("-----BEGIN") && trimmed.contains("PRIVATE KEY-----") {
        return true;
    }
    // Check for database connection URL with embedded credentials
    if (trimmed.starts_with("postgres://")
        || trimmed.starts_with("postgresql://")
        || trimmed.starts_with("mysql://")
        || trimmed.starts_with("mongodb://")
        || trimmed.starts_with("redis://"))
        && trimmed.contains('@')
        && trimmed.contains(':')
    {
        return true;
    }
    // Check for long high-entropy tokens (>= 32 chars of pure alphanumeric/hex/base64 without spaces)
    if trimmed.len() >= 32
        && !trimmed.contains(' ')
        && !trimmed.contains('/')
        && !trimmed.contains('\\')
        && !trimmed.starts_with("http")
    {
        let is_alphanumeric_or_symbols = trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.' || c == '=');
        if is_alphanumeric_or_symbols
            && (trimmed.contains('_') || trimmed.contains('-') || trimmed.len() >= 40)
        {
            return true;
        }
    }
    false
}

/// Checks whether a key-value pair represents a secret.
pub fn is_secret(key: &str, value: &str) -> bool {
    is_secret_key(key) || is_secret_value(value)
}

/// Simple deterministic hash for secret masking correlation without revealing values.
fn simple_secret_hash(value: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in value.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{:08x}", (hash & 0xFFFFFFFF) as u32)
}

/// Masks an API key or sensitive token value directly using partial masking.
pub fn mask_api_key(value: &str) -> String {
    mask_secret_value("API_KEY", value, MaskStyle::Partial)
}

/// Masks a secret value according to the specified style.
pub fn mask_secret_value(key: &str, value: &str, style: MaskStyle) -> String {
    if !is_secret(key, value) && style == MaskStyle::Partial {
        return value.to_string();
    }

    match style {
        MaskStyle::Full => "********".to_string(),
        MaskStyle::TypeOnly => format!("[SECRET: length {}]", value.len()),
        MaskStyle::Hash => format!("[SECRET: #{}]", simple_secret_hash(value)),
        MaskStyle::Partial => {
            // Check for database URL with credentials
            if (value.starts_with("postgres://")
                || value.starts_with("postgresql://")
                || value.starts_with("mysql://")
                || value.starts_with("mongodb://")
                || value.starts_with("redis://"))
                && value.contains('@')
            {
                if let Some(at_idx) = value.find('@') {
                    if let Some(colon_idx) = value[..at_idx].rfind(':') {
                        let before_pass = &value[..colon_idx + 1];
                        let after_pass = &value[at_idx..];
                        return format!("{}***{}", before_pass, after_pass);
                    }
                }
            }

            let len = value.chars().count();
            if len <= 6 {
                "***".to_string()
            } else if len <= 12 {
                let first2: String = value.chars().take(2).collect();
                let last2: String = value.chars().skip(len - 2).collect();
                format!("{}***{}", first2, last2)
            } else {
                // Check if starts with a known prefix
                for prefix in SENSITIVE_VALUE_PREFIXES {
                    if value.starts_with(prefix) {
                        let prefix_len = prefix.chars().count();
                        let keep_lead = (prefix_len + 3).min(len.saturating_sub(6));
                        let first: String = value.chars().take(keep_lead).collect();
                        let last4: String = value.chars().skip(len - 4).collect();
                        return format!("{}...***...{}", first, last4);
                    }
                }
                let first3: String = value.chars().take(3).collect();
                let last4: String = value.chars().skip(len - 4).collect();
                format!("{}...***...{}", first3, last4)
            }
        }
    }
}

/// Sanitizes arbitrary text (e.g. log lines or error traces) by masking recognized secret tokens
/// such as `sk-...`, `Bearer ...`, `ghp_...`, PEM keys, and high-entropy API keys.
pub fn sanitize_text_secrets(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }

    let mut result = text.to_string();

    // Fast check if string might contain secrets
    let has_secret_indication = result.contains("sk-")
        || result.contains("xai-")
        || result.contains("ghp_")
        || result.contains("glpat-")
        || result.contains("AIza")
        || result.contains("AKIA")
        || result.contains("Bearer ")
        || result.contains("bearer ")
        || result.contains("-----BEGIN");

    if !has_secret_indication {
        return result;
    }

    // Replace Bearer tokens
    if let Some(bearer_idx) = result.find("Bearer ").or_else(|| result.find("bearer ")) {
        let token_start = bearer_idx + 7;
        let rest = &result[token_start..];
        let token_len = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-' || *c == '.')
            .count();
        if token_len >= 16 {
            let raw_token: String = rest.chars().take(token_len).collect();
            let masked = mask_secret_value("BEARER_TOKEN", &raw_token, MaskStyle::Partial);
            result = result.replace(&raw_token, &masked);
        }
    }

    // Scan for sk-..., xai-..., ghp_..., glpat-..., etc.
    for prefix in SENSITIVE_VALUE_PREFIXES {
        let p = *prefix;
        let mut search_from = 0;
        while let Some(found_idx) = result[search_from..].find(p) {
            let actual_idx = search_from + found_idx;
            let rest = &result[actual_idx..];
            let token_chars: Vec<char> = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-' || *c == '.')
                .collect();
            let token_len = token_chars.len();
            if token_len >= 16 {
                let token_str: String = token_chars.into_iter().collect();
                let masked = mask_secret_value("API_KEY", &token_str, MaskStyle::Partial);
                result = result.replace(&token_str, &masked);
                search_from = actual_idx + masked.len();
            } else {
                search_from = actual_idx + p.len();
            }
            if search_from >= result.len() {
                break;
            }
        }
    }

    result
}

/// Unescapes escape sequences in a double-quoted string.
fn unescape_double_quoted_str(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(&next) = chars.peek() {
                match next {
                    'n' => {
                        chars.next();
                        out.push('\n');
                    }
                    'r' => {
                        chars.next();
                        out.push('\r');
                    }
                    't' => {
                        chars.next();
                        out.push('\t');
                    }
                    '\\' => {
                        chars.next();
                        out.push('\\');
                    }
                    '"' => {
                        chars.next();
                        out.push('"');
                    }
                    '\'' => {
                        chars.next();
                        out.push('\'');
                    }
                    '$' => {
                        chars.next();
                        out.push('$');
                    }
                    '0' => {
                        chars.next();
                        out.push('\0');
                    }
                    'u' => {
                        chars.next();
                        // Parse \uXXXX unicode escape
                        let mut hex = String::new();
                        for _ in 0..4 {
                            if let Some(&hc) = chars.peek() {
                                if hc.is_ascii_hexdigit() {
                                    hex.push(hc);
                                    chars.next();
                                } else {
                                    break;
                                }
                            }
                        }
                        if hex.len() == 4 {
                            if let Ok(code) = u32::from_str_radix(&hex, 16) {
                                if let Some(ch) = char::from_u32(code) {
                                    out.push(ch);
                                    continue;
                                }
                            }
                        }
                        out.push_str("\\u");
                        out.push_str(&hex);
                    }
                    _ => {
                        out.push(c);
                    }
                }
            } else {
                out.push(c);
            }
        } else {
            out.push(c);
        }
    }

    out
}

/// Token representing a parsed segment for variable expansion.
#[derive(Debug, PartialEq, Eq)]
enum ExpandToken {
    Literal(String),
    VarSimple(String),
    VarBraced {
        name: String,
        op: Option<ExpansionOp>,
    },
}

#[derive(Debug, PartialEq, Eq)]
enum ExpansionOp {
    /// `${VAR:-default}` or `${VAR-default}`
    Default { value: String, check_empty: bool },
    /// `${VAR:=default}` or `${VAR=default}`
    AssignDefault { value: String, check_empty: bool },
    /// `${VAR:+alternate}` or `${VAR+alternate}`
    Alternate { value: String, check_empty: bool },
    /// `${VAR:?error_msg}` or `${VAR?error_msg}`
    Error { message: String, check_empty: bool },
}

/// Parses a string into variable expansion tokens.
fn tokenize_expansion(raw: &str) -> Vec<ExpandToken> {
    let mut tokens = Vec::new();
    let mut literal_buf = String::new();
    let mut chars = raw.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '$' {
            match chars.peek() {
                Some('$') => {
                    // Escaped `$$` -> literal `$`
                    chars.next();
                    literal_buf.push('$');
                }
                Some('{') => {
                    chars.next(); // Consume '{'
                    if !literal_buf.is_empty() {
                        tokens.push(ExpandToken::Literal(std::mem::take(&mut literal_buf)));
                    }

                    // Parse inside `${...}`
                    let mut inner = String::new();
                    let mut brace_depth = 1;

                    while let Some(ic) = chars.next() {
                        if ic == '{' {
                            brace_depth += 1;
                            inner.push(ic);
                        } else if ic == '}' {
                            brace_depth -= 1;
                            if brace_depth == 0 {
                                break;
                            }
                            inner.push(ic);
                        } else {
                            inner.push(ic);
                        }
                    }

                    // Parse operators inside inner
                    tokens.push(parse_braced_expression(&inner));
                }
                Some(&next_ch) if next_ch.is_ascii_alphabetic() || next_ch == '_' => {
                    if !literal_buf.is_empty() {
                        tokens.push(ExpandToken::Literal(std::mem::take(&mut literal_buf)));
                    }
                    let mut var_name = String::new();
                    while let Some(&vc) = chars.peek() {
                        if vc.is_ascii_alphanumeric() || vc == '_' {
                            var_name.push(vc);
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    tokens.push(ExpandToken::VarSimple(var_name));
                }
                _ => {
                    literal_buf.push('$');
                }
            }
        } else if c == '\\' {
            if chars.peek() == Some(&'$') {
                chars.next();
                literal_buf.push('$');
            } else {
                literal_buf.push(c);
            }
        } else {
            literal_buf.push(c);
        }
    }

    if !literal_buf.is_empty() {
        tokens.push(ExpandToken::Literal(literal_buf));
    }

    tokens
}

/// Parses an expression inside `${...}` such as `VAR:-default`, `VAR:=default`, `VAR:+alt`, `VAR:?err`.
fn parse_braced_expression(inner: &str) -> ExpandToken {
    // Check for operators: `:-`, `-`, `:=`, `=`, `:+`, `+`, `:?`, `?`
    let ops = [
        (":-", true, false, false, false), // (delim, check_empty, is_assign, is_alt, is_err)
        ("-", false, false, false, false),
        (":=", true, true, false, false),
        ("=", false, true, false, false),
        (":+", true, false, true, false),
        ("+", false, false, true, false),
        (":?", true, false, false, true),
        ("?", false, false, false, true),
    ];

    for (delim, check_empty, is_assign, is_alt, is_err) in ops {
        if let Some(idx) = inner.find(delim) {
            let name = inner[..idx].trim().to_string();
            let operand = inner[idx + delim.len()..].to_string();

            let op = if is_err {
                ExpansionOp::Error {
                    message: operand,
                    check_empty,
                }
            } else if is_alt {
                ExpansionOp::Alternate {
                    value: operand,
                    check_empty,
                }
            } else if is_assign {
                ExpansionOp::AssignDefault {
                    value: operand,
                    check_empty,
                }
            } else {
                ExpansionOp::Default {
                    value: operand,
                    check_empty,
                }
            };

            return ExpandToken::VarBraced {
                name,
                op: Some(op),
            };
        }
    }

    ExpandToken::VarBraced {
        name: inner.trim().to_string(),
        op: None,
    }
}

/// Expands variables in a raw string against local context and system environment.
pub fn expand_variables(raw: &str, context: &HashMap<String, String>) -> Result<String, EnvError> {
    let mut active_stack = Vec::new();
    let mut working_context = context.clone();
    expand_variables_internal(raw, &mut working_context, &mut active_stack, 0)
}

/// Internal recursive helper for variable expansion with cycle detection and depth checks.
fn expand_variables_internal(
    raw: &str,
    context: &mut HashMap<String, String>,
    active_stack: &mut Vec<String>,
    depth: usize,
) -> Result<String, EnvError> {
    if depth > MAX_EXPANSION_DEPTH {
        let last_var = active_stack.last().cloned().unwrap_or_default();
        return Err(EnvError::DepthLimitExceeded {
            variable: last_var,
            max_depth: MAX_EXPANSION_DEPTH,
        });
    }

    let tokens = tokenize_expansion(raw);
    let mut result = String::new();

    for token in tokens {
        match token {
            ExpandToken::Literal(lit) => {
                result.push_str(&lit);
            }
            ExpandToken::VarSimple(name) => {
                let resolved = resolve_variable_value(&name, context, active_stack, depth)?;
                result.push_str(&resolved);
            }
            ExpandToken::VarBraced { name, op } => {
                let (has_var, val_opt) = get_variable_raw(&name, context);

                match op {
                    None => {
                        let resolved =
                            resolve_variable_value(&name, context, active_stack, depth)?;
                        result.push_str(&resolved);
                    }
                    Some(ExpansionOp::Default { value, check_empty }) => {
                        let is_unset_or_empty = if check_empty {
                            val_opt.as_deref().unwrap_or("").is_empty()
                        } else {
                            !has_var
                        };

                        if is_unset_or_empty {
                            let expanded_def = expand_variables_internal(
                                &value,
                                context,
                                active_stack,
                                depth + 1,
                            )?;
                            result.push_str(&expanded_def);
                        } else {
                            let resolved =
                                resolve_variable_value(&name, context, active_stack, depth)?;
                            result.push_str(&resolved);
                        }
                    }
                    Some(ExpansionOp::AssignDefault { value, check_empty }) => {
                        let is_unset_or_empty = if check_empty {
                            val_opt.as_deref().unwrap_or("").is_empty()
                        } else {
                            !has_var
                        };

                        if is_unset_or_empty {
                            let expanded_def = expand_variables_internal(
                                &value,
                                context,
                                active_stack,
                                depth + 1,
                            )?;
                            context.insert(name.clone(), expanded_def.clone());
                            result.push_str(&expanded_def);
                        } else {
                            let resolved =
                                resolve_variable_value(&name, context, active_stack, depth)?;
                            result.push_str(&resolved);
                        }
                    }
                    Some(ExpansionOp::Alternate { value, check_empty }) => {
                        let is_set_and_valid = if check_empty {
                            !val_opt.as_deref().unwrap_or("").is_empty()
                        } else {
                            has_var
                        };

                        if is_set_and_valid {
                            let expanded_alt = expand_variables_internal(
                                &value,
                                context,
                                active_stack,
                                depth + 1,
                            )?;
                            result.push_str(&expanded_alt);
                        }
                    }
                    Some(ExpansionOp::Error {
                        message,
                        check_empty,
                    }) => {
                        let is_invalid = if check_empty {
                            val_opt.as_deref().unwrap_or("").is_empty()
                        } else {
                            !has_var
                        };

                        if is_invalid {
                            let msg = if message.trim().is_empty() {
                                format!("parameter '{}' is unset or null", name)
                            } else {
                                message
                            };
                            return Err(EnvError::UndefinedVariable {
                                variable: name,
                                message: msg,
                                line: None,
                            });
                        } else {
                            let resolved =
                                resolve_variable_value(&name, context, active_stack, depth)?;
                            result.push_str(&resolved);
                        }
                    }
                }
            }
        }
    }

    Ok(result)
}

/// Helper to get raw variable status from context or std::env.
fn get_variable_raw(name: &str, context: &HashMap<String, String>) -> (bool, Option<String>) {
    if let Some(val) = context.get(name) {
        (true, Some(val.clone()))
    } else if let Ok(val) = std::env::var(name) {
        (true, Some(val))
    } else {
        (false, None)
    }
}

/// Resolves a single variable value, checking for recursion cycles.
fn resolve_variable_value(
    name: &str,
    context: &mut HashMap<String, String>,
    active_stack: &mut Vec<String>,
    depth: usize,
) -> Result<String, EnvError> {
    if active_stack.contains(&name.to_string()) {
        let mut cycle_path = active_stack.clone();
        cycle_path.push(name.to_string());
        return Err(EnvError::CyclicVariable {
            variable: name.to_string(),
            path: cycle_path,
        });
    }

    let val = if let Some(v) = context.get(name) {
        v.clone()
    } else if let Ok(v) = std::env::var(name) {
        v
    } else {
        return Ok(String::new());
    };

    if val.contains('$') {
        active_stack.push(name.to_string());
        let expanded = expand_variables_internal(&val, context, active_stack, depth + 1)?;
        active_stack.pop();
        Ok(expanded)
    } else {
        Ok(val)
    }
}

/// Parses the content of a `.env` file into key-value entries.
pub fn parse_env_str(content: &str) -> Result<HashMap<String, String>, EnvError> {
    let raw_entries = parse_raw_entries(content, EnvSource::Inline)?;
    let mut map = HashMap::with_capacity(raw_entries.len());

    for entry in raw_entries {
        map.insert(entry.key, entry.value);
    }

    Ok(map)
}

/// Internal entry parser supporting multiline quotes, inline comments, and initial expansion context.
fn parse_raw_entries(content: &str, source: EnvSource) -> Result<Vec<EnvVariable>, EnvError> {
    parse_raw_entries_with_context(content, source, None)
}

/// Internal entry parser with optional parent context for variable expansion across cascading files.
fn parse_raw_entries_with_context(
    content: &str,
    source: EnvSource,
    parent_context: Option<&HashMap<String, String>>,
) -> Result<Vec<EnvVariable>, EnvError> {
    let mut entries = Vec::new();
    let mut context_map = parent_context.cloned().unwrap_or_default();

    // Strip UTF-8 BOM if present
    let clean_content = content.strip_prefix('\u{feff}').unwrap_or(content);
    let mut lines = clean_content.lines().enumerate().peekable();

    while let Some((line_idx, line)) = lines.next() {
        let line_num = line_idx + 1;
        let trimmed = line.trim();

        // Skip empty lines and full-line comments
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // Handle `export ` prefix (e.g., `export FOO=BAR`)
        let line_to_parse = if let Some(rest) = trimmed.strip_prefix("export") {
            if rest.starts_with(char::is_whitespace) {
                rest.trim_start()
            } else {
                trimmed
            }
        } else {
            trimmed
        };

        // Find the first `=` delimiter
        let equal_pos = match line_to_parse.find('=') {
            Some(pos) => pos,
            None => {
                return Err(EnvError::Parse {
                    line: line_num,
                    message: format!(
                        "Expected 'KEY=VALUE' format, found: '{}'",
                        line_to_parse
                    ),
                });
            }
        };

        let raw_key = line_to_parse[..equal_pos].trim();
        if raw_key.is_empty() {
            return Err(EnvError::InvalidKey {
                line: line_num,
                key: String::new(),
            });
        }

        // Validate key characters (letters, numbers, underscores, dots, hyphens)
        if !raw_key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-')
        {
            return Err(EnvError::InvalidKey {
                line: line_num,
                key: raw_key.to_string(),
            });
        }

        let key = raw_key.to_string();
        let val_part = line_to_parse[equal_pos + 1..].trim_start();

        // Parse value: Single Quoted, Double Quoted, or Unquoted
        let (val_str, is_single_quoted) = if let Some(rest) = val_part.strip_prefix('\'') {
            // Single-quoted value
            let mut val_acc = String::new();
            let mut closed = false;

            if let Some(close_idx) = rest.find('\'') {
                val_acc.push_str(&rest[..close_idx]);
                closed = true;
            } else {
                val_acc.push_str(rest);
                // Read subsequent lines until closing `'` is found
                while let Some((_next_idx, next_line)) = lines.next() {
                    val_acc.push('\n');
                    if let Some(close_idx) = next_line.find('\'') {
                        val_acc.push_str(&next_line[..close_idx]);
                        closed = true;
                        break;
                    } else {
                        val_acc.push_str(next_line);
                    }
                }
            }

            if !closed {
                return Err(EnvError::UnclosedQuote {
                    line: line_num,
                    quote_char: '\'',
                });
            }

            (val_acc, true)
        } else if let Some(rest) = val_part.strip_prefix('"') {
            // Double-quoted value
            let mut val_acc = String::new();
            let mut closed = false;

            // Search for unescaped closing `"`
            let mut search_in = rest;
            loop {
                if let Some(close_idx) = find_unescaped_quote(search_in, '"') {
                    val_acc.push_str(&search_in[..close_idx]);
                    closed = true;
                    break;
                } else {
                    val_acc.push_str(search_in);
                    if let Some((_, next_line)) = lines.next() {
                        val_acc.push('\n');
                        search_in = next_line;
                    } else {
                        break;
                    }
                }
            }

            if !closed {
                return Err(EnvError::UnclosedQuote {
                    line: line_num,
                    quote_char: '"',
                });
            }

            // Unescape double-quoted content
            let unescaped = unescape_double_quoted_str(&val_acc);
            (unescaped, false)
        } else {
            // Unquoted value: Strip inline comments (# preceded by whitespace)
            let mut end_idx = val_part.len();
            let mut chars = val_part.char_indices().peekable();

            while let Some((idx, c)) = chars.next() {
                if c == '#' {
                    // If at the start or preceded by whitespace, it's an inline comment
                    if idx == 0 || val_part[..idx].ends_with(char::is_whitespace) {
                        end_idx = idx;
                        break;
                    }
                }
            }

            let unquoted_val = val_part[..end_idx].trim_end();
            let unescaped = unescape_double_quoted_str(unquoted_val);
            (unescaped, false)
        };

        // Variable expansion (single-quoted values are not expanded)
        let final_value = if is_single_quoted {
            val_str
        } else {
            expand_variables_internal(&val_str, &mut context_map, &mut Vec::new(), 0)?
        };

        context_map.insert(key.clone(), final_value.clone());

        let env_var = EnvVariable::new(key, final_value, source.clone()).with_line(line_num);
        entries.push(env_var);
    }

    Ok(entries)
}

/// Finds the index of the first unescaped quote character in a string.
fn find_unescaped_quote(s: &str, quote: char) -> Option<usize> {
    let mut escaped = false;
    for (i, c) in s.char_indices() {
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == quote {
            return Some(i);
        }
    }
    None
}

/// Parses JSON configuration content (e.g. `config.json`) into environment variables.
pub fn parse_json_config_str(
    content: &str,
    source: EnvSource,
) -> Result<Vec<EnvVariable>, EnvError> {
    let value: serde_json::Value = serde_json::from_str(content).map_err(|e| EnvError::Parse {
        line: 1,
        message: format!("Invalid JSON configuration: {}", e),
    })?;

    let mut entries = Vec::new();

    if let serde_json::Value::Object(map) = value {
        for (key, val) in map {
            if key == "env" {
                // If there is a nested "env" object
                if let serde_json::Value::Object(env_map) = val {
                    for (env_k, env_v) in env_map {
                        let v_str = match env_v {
                            serde_json::Value::String(s) => s,
                            serde_json::Value::Bool(b) => b.to_string(),
                            serde_json::Value::Number(n) => n.to_string(),
                            serde_json::Value::Null => continue,
                            other => other.to_string(),
                        };
                        entries.push(EnvVariable::new(env_k, v_str, source.clone()));
                    }
                }
            } else {
                let v_str = match val {
                    serde_json::Value::String(s) => s,
                    serde_json::Value::Bool(b) => b.to_string(),
                    serde_json::Value::Number(n) => n.to_string(),
                    serde_json::Value::Null => continue,
                    other => other.to_string(),
                };

                // Add original key
                entries.push(EnvVariable::new(&key, &v_str, source.clone()));

                // Add uppercase alias (e.g. "deepseek_api_key" -> "DEEPSEEK_API_KEY", "model" -> "MODEL")
                let upper_key = key.to_ascii_uppercase();
                if upper_key != key {
                    entries.push(EnvVariable::new(upper_key, &v_str, source.clone()));
                }
            }
        }
    }

    Ok(entries)
}

/// Returns the list of standard global configuration file paths in priority order.
/// 1. $FUSION_GLOBAL_CONFIG (if set)
/// 2. $XDG_CONFIG_HOME/fusion/config.json (if set)
/// 3. ~/.config/fusion/config.json
/// 4. ~/.fusion/config.json
pub fn global_config_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    if let Ok(env_path) = std::env::var("FUSION_GLOBAL_CONFIG") {
        if !env_path.trim().is_empty() {
            paths.push(PathBuf::from(env_path));
        }
    }

    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.trim().is_empty() {
            let p = PathBuf::from(xdg).join("fusion").join("config.json");
            if !paths.contains(&p) {
                paths.push(p);
            }
        }
    }

    if let Some(config_dir) = dirs::config_dir() {
        let p = config_dir.join("fusion").join("config.json");
        if !paths.contains(&p) {
            paths.push(p);
        }
    }

    if let Some(home) = dirs::home_dir() {
        let p1 = home.join(".config").join("fusion").join("config.json");
        if !paths.contains(&p1) {
            paths.push(p1);
        }
        let p2 = home.join(".fusion").join("config.json");
        if !paths.contains(&p2) {
            paths.push(p2);
        }
    }

    paths
}

/// Discovers the first existing global configuration file, if any.
pub fn find_global_config_file() -> Option<PathBuf> {
    for path in global_config_paths() {
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

/// Returns standard project configuration file paths for a directory.
/// 1. $FUSION_PROJECT_CONFIG (if set)
/// 2. <dir>/.fusion/config.json
/// 3. <dir>/fusion.json
/// 4. <dir>/.fusion.json
pub fn project_config_paths(dir: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();

    if let Ok(env_path) = std::env::var("FUSION_PROJECT_CONFIG") {
        if !env_path.trim().is_empty() {
            paths.push(PathBuf::from(env_path));
        }
    }

    paths.push(dir.join(".fusion").join("config.json"));
    paths.push(dir.join("fusion.json"));
    paths.push(dir.join(".fusion.json"));

    paths
}

/// Discovers a project configuration file in the specified directory or its parents.
pub fn find_project_config_file(start: &Path, search_parents: bool) -> Option<PathBuf> {
    let mut current = start.to_path_buf();
    loop {
        for candidate in project_config_paths(&current) {
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        if !search_parents || !current.pop() {
            break;
        }
    }
    None
}

/// A container of loaded environment variables with secret tracking,
/// masking, and log sanitization capabilities.
#[derive(Clone)]
pub struct LoadedEnv {
    /// Map of variable name to `EnvVariable`.
    entries: HashMap<String, EnvVariable>,
    /// List of paths from which environment variables were loaded.
    loaded_files: Vec<PathBuf>,
    /// Custom secret key names.
    custom_secret_keys: HashSet<String>,
    /// Custom secret patterns.
    custom_secret_patterns: Vec<String>,
    /// Configured masking style.
    mask_style: MaskStyle,
}

impl LoadedEnv {
    /// Create a new empty `LoadedEnv`.
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            loaded_files: Vec::new(),
            custom_secret_keys: HashSet::new(),
            custom_secret_patterns: Vec::new(),
            mask_style: MaskStyle::default(),
        }
    }

    /// Set the default masking style.
    pub fn with_mask_style(mut self, style: MaskStyle) -> Self {
        self.mask_style = style;
        self
    }

    /// Add custom secret keys.
    pub fn with_custom_secret_keys(
        mut self,
        keys: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        for k in keys {
            self.custom_secret_keys
                .insert(k.into().to_ascii_uppercase());
        }
        self.recompute_secrets();
        self
    }

    /// Add custom secret patterns.
    pub fn with_custom_secret_patterns(
        mut self,
        patterns: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        for p in patterns {
            self.custom_secret_patterns
                .push(p.into().to_ascii_uppercase());
        }
        self.recompute_secrets();
        self
    }

    /// Recompute secret flags for all entries based on known and custom secret rules.
    fn recompute_secrets(&mut self) {
        for (k, var) in self.entries.iter_mut() {
            let key_upper = k.to_ascii_uppercase();
            let mut is_sec =
                is_secret(k, &var.value) || self.custom_secret_keys.contains(&key_upper);
            if !is_sec {
                for pat in &self.custom_secret_patterns {
                    if key_upper.contains(pat) {
                        is_sec = true;
                        break;
                    }
                }
            }
            var.is_secret = is_sec;
        }
    }

    /// Inserts or unconditionally overwrites an environment variable entry.
    pub fn insert(&mut self, mut var: EnvVariable) {
        let key_upper = var.key.to_ascii_uppercase();
        let mut is_sec = var.is_secret
            || is_secret(&var.key, &var.value)
            || self.custom_secret_keys.contains(&key_upper);
        if !is_sec {
            for pat in &self.custom_secret_patterns {
                if key_upper.contains(pat) {
                    is_sec = true;
                    break;
                }
            }
        }
        var.is_secret = is_sec;
        self.entries.insert(var.key.clone(), var);
    }

    /// Inserts an environment variable respecting the multi-tier hierarchy.
    /// Overwrites existing entries only if the incoming variable's tier is greater than or equal to the existing entry's tier.
    /// Returns `true` if the entry was inserted or updated.
    pub fn insert_tiered(&mut self, var: EnvVariable) -> bool {
        if let Some(existing) = self.entries.get(&var.key) {
            if var.tier() < existing.tier() {
                return false;
            }
        }
        self.insert(var);
        true
    }

    /// Returns the resolved value for the specified environment variable key.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries.get(key).map(|v| v.value.as_str())
    }

    /// Returns the value or a default if not found.
    pub fn get_or<'a>(&'a self, key: &str, default: &'a str) -> &'a str {
        self.get(key).unwrap_or(default)
    }

    /// Returns the first non-empty value among multiple candidate keys, or a fallback default.
    pub fn get_with_fallback(&self, keys: &[&str], default: Option<&str>) -> Option<String> {
        for k in keys {
            if let Some(val) = self.get(k) {
                if !val.trim().is_empty() {
                    return Some(val.to_string());
                }
            }
        }
        default.map(|d| d.to_string())
    }

    /// Returns the full `EnvVariable` entry for a key if present.
    pub fn get_entry(&self, key: &str) -> Option<&EnvVariable> {
        self.entries.get(key)
    }

    /// Returns the source origin of a key if present.
    pub fn get_source(&self, key: &str) -> Option<&EnvSource> {
        self.entries.get(key).map(|v| &v.source)
    }

    /// Returns the hierarchy tier of a key if present.
    pub fn get_tier(&self, key: &str) -> Option<HierarchyTier> {
        self.entries.get(key).map(|v| v.tier())
    }

    /// Returns the masked value for the specified key.
    pub fn get_masked(&self, key: &str) -> Option<String> {
        self.entries
            .get(key)
            .map(|v| v.masked_value(self.mask_style))
    }

    /// Returns the parsed boolean value for a key (interpreting "1", "true", "yes", "on").
    pub fn get_bool(&self, key: &str) -> Option<bool> {
        self.get(key).and_then(|v| {
            let lower = v.trim().to_ascii_lowercase();
            match lower.as_str() {
                "1" | "true" | "yes" | "on" => Some(true),
                "0" | "false" | "no" | "off" => Some(false),
                _ => None,
            }
        })
    }

    /// Returns the parsed integer or numeric value for a key.
    pub fn get_int<T: FromStr>(&self, key: &str) -> Option<T> {
        self.get(key).and_then(|v| v.trim().parse::<T>().ok())
    }

    /// Returns true if the variable key exists in this environment.
    pub fn contains_key(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }

    /// Returns true if the specified key is considered sensitive/secret.
    pub fn is_secret(&self, key: &str) -> bool {
        self.entries
            .get(key)
            .map(|v| v.is_secret)
            .unwrap_or_else(|| is_secret_key(key))
    }

    /// Returns the number of loaded environment variables.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns true if no environment variables are loaded.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns an iterator over `(&key, &value)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.entries
            .iter()
            .map(|(k, v)| (k.as_str(), v.value.as_str()))
    }

    /// Returns an iterator over `(&key, masked_value)` pairs.
    pub fn iter_masked(&self) -> impl Iterator<Item = (&str, String)> + '_ {
        self.entries
            .iter()
            .map(move |(k, v)| (k.as_str(), v.masked_value(self.mask_style)))
    }

    /// Exports all variables as a standard `HashMap<String, String>`.
    pub fn to_hash_map(&self) -> HashMap<String, String> {
        self.entries
            .iter()
            .map(|(k, v)| (k.clone(), v.value.clone()))
            .collect()
    }

    /// Exports all variables as a `HashMap<String, String>` with secrets masked.
    pub fn to_masked_map(&self) -> HashMap<String, String> {
        self.entries
            .iter()
            .map(|(k, v)| (k.clone(), v.masked_value(self.mask_style)))
            .collect()
    }

    /// Returns a list of paths from which environment variables were loaded.
    pub fn loaded_files(&self) -> &[PathBuf] {
        &self.loaded_files
    }

    /// Returns a list of all variable keys marked as secret.
    pub fn secret_keys(&self) -> Vec<&str> {
        let mut keys: Vec<&str> = self
            .entries
            .iter()
            .filter(|(_, v)| v.is_secret)
            .map(|(k, _)| k.as_str())
            .collect();
        keys.sort_unstable();
        keys
    }

    /// Safely applies all loaded variables to `std::env`.
    ///
    /// If `override_existing` is `false`, already-set process environment variables
    /// will NOT be overwritten.
    /// Returns the number of environment variables set.
    pub fn apply(&self, override_existing: bool) -> usize {
        let mut count = 0;
        for (k, v) in &self.entries {
            if override_existing || std::env::var_os(k).is_none() {
                std::env::set_var(k, &v.value);
                count += 1;
            }
        }
        count
    }

    /// Sanitizes an arbitrary log message or string by replacing all known loaded secret values with masks,
    /// and running general secret pattern masking.
    pub fn sanitize_text(&self, text: &str) -> String {
        let mut sanitized = text.to_string();
        for (_, var) in &self.entries {
            if var.is_secret && !var.value.is_empty() && var.value.len() >= 4 {
                let mask = var.masked_value(self.mask_style);
                sanitized = sanitized.replace(&var.value, &mask);
            }
        }
        sanitize_text_secrets(&sanitized)
    }

    /// Expands variables in a raw template string using the loaded environment context.
    pub fn resolve_expanded(&self, raw: &str) -> Result<String, EnvError> {
        expand_variables(raw, &self.to_hash_map())
    }

    /// Formats the loaded variables as a `.env` format string.
    ///
    /// If `mask_secrets` is true, sensitive values are replaced with their masked representations.
    pub fn format_env_file(&self, mask_secrets: bool) -> String {
        let mut lines = Vec::with_capacity(self.entries.len());
        let mut sorted_entries: Vec<_> = self.entries.values().collect();
        sorted_entries.sort_by_key(|v| &v.key);

        for var in sorted_entries {
            let val = if mask_secrets && var.is_secret {
                var.masked_value(self.mask_style)
            } else {
                var.value.clone()
            };

            // Quote if contains spaces, newlines, or special characters
            if val.contains('\n') || val.contains(' ') || val.contains('"') || val.contains('#') {
                let escaped = val
                    .replace('\\', "\\\\")
                    .replace('"', "\\\"")
                    .replace('\n', "\\n")
                    .replace('\r', "\\r");
                lines.push(format!("{}=\"{}\"", var.key, escaped));
            } else {
                lines.push(format!("{}={}", var.key, val));
            }
        }

        lines.join("\n")
    }

    /// Formats a clean ASCII summary table of the loaded environment variables with masked secrets.
    pub fn format_table(&self) -> String {
        if self.entries.is_empty() {
            return "No environment variables loaded.".to_string();
        }

        let mut sorted: Vec<_> = self.entries.values().collect();
        sorted.sort_by_key(|v| &v.key);

        let max_key_len = sorted.iter().map(|v| v.key.len()).max().unwrap_or(3).max(3);
        let max_src_len = sorted
            .iter()
            .map(|v| v.source.to_string().len())
            .max()
            .unwrap_or(6)
            .max(6);

        let mut out = String::new();
        out.push_str(&format!(
            "┌─{:─<key_w$}─┬─{:─<src_w$}─┬─{:─<8}─┬─{:─<30}─┐\n",
            "",
            "",
            "",
            "",
            key_w = max_key_len,
            src_w = max_src_len
        ));
        out.push_str(&format!(
            "│ {:<key_w$} │ {:<src_w$} │ {:<8} │ {:<30} │\n",
            "KEY",
            "SOURCE",
            "SECRET",
            "VALUE (MASKED)",
            key_w = max_key_len,
            src_w = max_src_len
        ));
        out.push_str(&format!(
            "├─{:─<key_w$}─┼─{:─<src_w$}─┼─{:─<8}─┼─{:─<30}─┤\n",
            "",
            "",
            "",
            "",
            key_w = max_key_len,
            src_w = max_src_len
        ));

        for var in sorted {
            let secret_str = if var.is_secret { "YES" } else { "NO" };
            let masked = var.masked_value(self.mask_style);
            let display_val = if masked.len() > 30 {
                format!("{}...", &masked[..27])
            } else {
                masked
            };

            out.push_str(&format!(
                "│ {:<key_w$} │ {:<src_w$} │ {:<8} │ {:<30} │\n",
                var.key,
                var.source.to_string(),
                secret_str,
                display_val,
                key_w = max_key_len,
                src_w = max_src_len
            ));
        }

        out.push_str(&format!(
            "└─{:─<key_w$}─┴─{:─<src_w$}─┴─{:─<8}─┴─{:─<30}─┘",
            "",
            "",
            "",
            "",
            key_w = max_key_len,
            src_w = max_src_len
        ));
        out
    }
}

impl Default for LoadedEnv {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for LoadedEnv {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut d = f.debug_map();
        let mut sorted_keys: Vec<_> = self.entries.keys().collect();
        sorted_keys.sort();

        for k in sorted_keys {
            if let Some(var) = self.entries.get(k) {
                d.entry(k, &var.masked_value(self.mask_style));
            }
        }

        d.finish()
    }
}

impl fmt::Display for LoadedEnv {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.format_env_file(true))
    }
}

/// Builder for discovering, parsing, and resolving multi-tier configuration and `.env` files.
///
/// Hierarchy Precedence (Higher overrides Lower):
/// 1. CLI Flags (`EnvSource::Cli`, Tier 5)
/// 2. Process Environment (`EnvSource::Process`, Tier 4)
/// 3. Local `.env` Files (`EnvSource::LocalEnv`, Tier 3: `.env` -> `.env.local` -> `.env.<mode>` -> `.env.<mode>.local`)
/// 4. Project Configuration File (`EnvSource::ProjectConfig`, Tier 2: `./.fusion/config.json` or `./fusion.json`)
/// 5. Global User Configuration (`EnvSource::GlobalConfig`, Tier 1: `~/.config/fusion/config.json` or `~/.fusion/config.json`)
#[derive(Debug, Clone)]
pub struct EnvLoader {
    /// Base directory to search for environment and project configuration files.
    directory: PathBuf,
    /// Explicit environment files to load in order (bypasses multi-tier cascade if set).
    explicit_files: Vec<PathBuf>,
    /// Environment mode name (e.g. "development", "test", "production").
    environment: Option<String>,
    /// Whether to enable automatic cascading (.env -> .env.local -> .env.<mode> -> .env.<mode>.local).
    cascade: bool,
    /// Whether to search parent directories if no `.env` is found in current directory.
    search_parents: bool,
    /// Whether loaded variables should override existing host process environment variables when applied.
    override_process: bool,
    /// Custom secret key names.
    custom_secret_keys: Vec<String>,
    /// Custom secret patterns.
    custom_secret_patterns: Vec<String>,
    /// Masking style for sensitive variables.
    mask_style: MaskStyle,
    /// CLI flag overrides (Tier 5: Highest priority).
    cli_flags: HashMap<String, String>,
    /// Whether to include host process environment (Tier 4).
    include_process_env: bool,
    /// Whether to include local .env files (Tier 3).
    include_local_dotenv: bool,
    /// Whether to include project config (Tier 2).
    include_project_config: bool,
    /// Explicit project config file path (overrides auto-discovery).
    project_config_path: Option<PathBuf>,
    /// Whether to include global config (Tier 1).
    include_global_config: bool,
    /// Explicit global config file path (overrides auto-discovery).
    global_config_path: Option<PathBuf>,
}

impl EnvLoader {
    /// Creates a new `EnvLoader` builder with current directory as the default base.
    pub fn new() -> Self {
        Self {
            directory: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            explicit_files: Vec::new(),
            environment: None,
            cascade: true,
            search_parents: false,
            override_process: false,
            custom_secret_keys: Vec::new(),
            custom_secret_patterns: Vec::new(),
            mask_style: MaskStyle::default(),
            cli_flags: HashMap::new(),
            include_process_env: false,
            include_local_dotenv: true,
            include_project_config: true,
            project_config_path: None,
            include_global_config: true,
            global_config_path: None,
        }
    }

    /// Sets the base directory for environment file discovery.
    pub fn directory(mut self, dir: impl AsRef<Path>) -> Self {
        self.directory = dir.as_ref().to_path_buf();
        self
    }

    /// Adds an explicit environment file to parse and load.
    pub fn file(mut self, file: impl AsRef<Path>) -> Self {
        self.explicit_files.push(file.as_ref().to_path_buf());
        self
    }

    /// Adds multiple explicit environment files to parse and load.
    pub fn files(mut self, files: impl IntoIterator<Item = impl AsRef<Path>>) -> Self {
        for f in files {
            self.explicit_files.push(f.as_ref().to_path_buf());
        }
        self
    }

    /// Sets the runtime environment mode (e.g. "development", "production", "test").
    pub fn environment(mut self, env: impl Into<String>) -> Self {
        self.environment = Some(env.into());
        self
    }

    /// Enables or disables cascading file loading (`.env` -> `.env.local` etc.).
    pub fn cascade(mut self, enable: bool) -> Self {
        self.cascade = enable;
        self
    }

    /// Enables or disables traversing parent directories to locate `.env` files.
    pub fn search_parents(mut self, search: bool) -> Self {
        self.search_parents = search;
        self
    }

    /// Configures whether loaded variables should overwrite existing process environment variables when applied.
    pub fn override_process(mut self, override_proc: bool) -> Self {
        self.override_process = override_proc;
        self
    }

    /// Configures the secret masking style.
    pub fn mask_style(mut self, style: MaskStyle) -> Self {
        self.mask_style = style;
        self
    }

    /// Adds a custom secret key name.
    pub fn custom_secret_key(mut self, key: impl Into<String>) -> Self {
        self.custom_secret_keys.push(key.into());
        self
    }

    /// Adds custom secret key patterns.
    pub fn custom_secret_patterns(
        mut self,
        patterns: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        for p in patterns {
            self.custom_secret_patterns.push(p.into());
        }
        self
    }

    /// Adds a CLI flag override (Tier 5: Highest priority).
    pub fn cli_flag(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.cli_flags.insert(key.into(), value.into());
        self
    }

    /// Adds multiple CLI flag overrides (Tier 5: Highest priority).
    pub fn cli_flags(
        mut self,
        flags: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
    ) -> Self {
        for (k, v) in flags {
            self.cli_flags.insert(k.into(), v.into());
        }
        self
    }

    /// Enables or disables incorporating host process environment variables (`std::env`).
    pub fn process_env(mut self, enable: bool) -> Self {
        self.include_process_env = enable;
        self
    }

    /// Alias for `process_env`.
    pub fn include_process_env(mut self, enable: bool) -> Self {
        self.include_process_env = enable;
        self
    }

    /// Enables or disables loading local `.env` files.
    pub fn local_dotenv(mut self, enable: bool) -> Self {
        self.include_local_dotenv = enable;
        self
    }

    /// Alias for `local_dotenv`.
    pub fn include_local_dotenv(mut self, enable: bool) -> Self {
        self.include_local_dotenv = enable;
        self
    }

    /// Enables or disables loading project configuration files (`.fusion/config.json`).
    pub fn project_config(mut self, enable: bool) -> Self {
        self.include_project_config = enable;
        self
    }

    /// Alias for `project_config`.
    pub fn include_project_config(mut self, enable: bool) -> Self {
        self.include_project_config = enable;
        self
    }

    /// Sets an explicit project configuration file path.
    pub fn project_config_file(mut self, path: impl AsRef<Path>) -> Self {
        self.project_config_path = Some(path.as_ref().to_path_buf());
        self.include_project_config = true;
        self
    }

    /// Enables or disables loading global configuration files (`~/.config/fusion/config.json`).
    pub fn global_config(mut self, enable: bool) -> Self {
        self.include_global_config = enable;
        self
    }

    /// Alias for `global_config`.
    pub fn include_global_config(mut self, enable: bool) -> Self {
        self.include_global_config = enable;
        self
    }

    /// Sets an explicit global configuration file path.
    pub fn global_config_file(mut self, path: impl AsRef<Path>) -> Self {
        self.global_config_path = Some(path.as_ref().to_path_buf());
        self.include_global_config = true;
        self
    }

    /// Parses and loads all tiers in precedence order:
    /// Global config (Tier 1) -> Project config (Tier 2) -> Local .env (Tier 3) -> Process env (Tier 4) -> CLI flags (Tier 5).
    pub fn load(&self) -> Result<LoadedEnv, EnvError> {
        let mut loaded = LoadedEnv::new()
            .with_mask_style(self.mask_style)
            .with_custom_secret_keys(self.custom_secret_keys.clone())
            .with_custom_secret_patterns(self.custom_secret_patterns.clone());

        // If explicit files are specified, load them directly
        if !self.explicit_files.is_empty() {
            for path in &self.explicit_files {
                let content = match std::fs::read_to_string(path) {
                    Ok(c) => c,
                    Err(e) => {
                        return Err(EnvError::Io {
                            path: path.clone(),
                            source: e,
                        });
                    }
                };
                let current_context = loaded.to_hash_map();
                let entries = parse_raw_entries_with_context(
                    &content,
                    EnvSource::File(path.clone()),
                    Some(&current_context),
                )?;
                for var in entries {
                    loaded.insert(var);
                }
                loaded.loaded_files.push(path.clone());
            }

            // Apply CLI flags on top if provided
            for (k, v) in &self.cli_flags {
                loaded.insert(EnvVariable::new(k.clone(), v.clone(), EnvSource::Cli));
            }

            return Ok(loaded);
        }

        // --- Tier 1: Global Configuration (`~/.config/fusion/config.json`) ---
        if self.include_global_config {
            let global_path = self
                .global_config_path
                .clone()
                .or_else(find_global_config_file);

            if let Some(path) = global_path {
                if path.is_file() {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        if let Ok(entries) =
                            parse_json_config_str(&content, EnvSource::GlobalConfig(path.clone()))
                        {
                            for var in entries {
                                loaded.insert_tiered(var);
                            }
                            loaded.loaded_files.push(path);
                        }
                    }
                }
            }
        }

        // --- Tier 2: Project Configuration (`./.fusion/config.json` or `./fusion.json`) ---
        if self.include_project_config {
            let project_path = self
                .project_config_path
                .clone()
                .or_else(|| find_project_config_file(&self.directory, self.search_parents));

            if let Some(path) = project_path {
                if path.is_file() {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        if let Ok(entries) =
                            parse_json_config_str(&content, EnvSource::ProjectConfig(path.clone()))
                        {
                            for var in entries {
                                loaded.insert_tiered(var);
                            }
                            loaded.loaded_files.push(path);
                        }
                    }
                }
            }
        }

        // --- Tier 3: Local `.env` Files (Cascading) ---
        if self.include_local_dotenv {
            let target_dir = if self.search_parents {
                find_env_dir(&self.directory).unwrap_or_else(|| self.directory.clone())
            } else {
                self.directory.clone()
            };

            let mut files_to_load = Vec::new();

            if self.cascade {
                // Cascading order (lowest priority to highest priority within Tier 3):
                // 1. .env
                // 2. .env.local
                // 3. .env.<environment>
                // 4. .env.<environment>.local
                let base_env = target_dir.join(".env");
                let local_env = target_dir.join(".env.local");

                if base_env.is_file() {
                    files_to_load.push(base_env);
                }
                if local_env.is_file() {
                    files_to_load.push(local_env);
                }

                if let Some(env_name) = &self.environment {
                    let env_specific = target_dir.join(format!(".env.{}", env_name));
                    let env_specific_local = target_dir.join(format!(".env.{}.local", env_name));

                    if env_specific.is_file() {
                        files_to_load.push(env_specific);
                    }
                    if env_specific_local.is_file() {
                        files_to_load.push(env_specific_local);
                    }
                }
            } else {
                let base_env = target_dir.join(".env");
                if base_env.is_file() {
                    files_to_load.push(base_env);
                }
            }

            for path in files_to_load {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    let current_context = loaded.to_hash_map();
                    let entries = parse_raw_entries_with_context(
                        &content,
                        EnvSource::LocalEnv(path.clone()),
                        Some(&current_context),
                    )?;
                    for var in entries {
                        loaded.insert_tiered(var);
                    }
                    loaded.loaded_files.push(path);
                }
            }
        }

        // --- Tier 4: Host Process Environment (`std::env`) ---
        if self.include_process_env {
            for (k, v) in std::env::vars() {
                loaded.insert_tiered(EnvVariable::new(k, v, EnvSource::Process));
            }
        }

        // --- Tier 5: CLI Flag Overrides (Highest Priority) ---
        for (k, v) in &self.cli_flags {
            loaded.insert_tiered(EnvVariable::new(k.clone(), v.clone(), EnvSource::Cli));
        }

        Ok(loaded)
    }

    /// Loads the complete 5-tier configuration hierarchy (including process environment).
    pub fn load_hierarchy(&self) -> Result<LoadedEnv, EnvError> {
        let mut clone = self.clone();
        clone.include_process_env = true;
        clone.include_global_config = true;
        clone.include_project_config = true;
        clone.include_local_dotenv = true;
        clone.load()
    }

    /// Loads environment files and applies them directly to `std::env`.
    pub fn load_and_apply(&self) -> Result<LoadedEnv, EnvError> {
        let loaded = self.load()?;
        loaded.apply(self.override_process);
        Ok(loaded)
    }

    /// Convenience helper to parse an in-memory `.env` content string into `LoadedEnv`.
    pub fn parse_str(content: &str) -> Result<LoadedEnv, EnvError> {
        let mut loaded = LoadedEnv::new();
        let entries = parse_raw_entries(content, EnvSource::Inline)?;
        for var in entries {
            loaded.insert(var);
        }
        Ok(loaded)
    }

    /// Convenience helper to parse a single `.env` file on disk.
    pub fn parse_file(path: impl AsRef<Path>) -> Result<LoadedEnv, EnvError> {
        let p = path.as_ref();
        let content = std::fs::read_to_string(p).map_err(|e| EnvError::Io {
            path: p.to_path_buf(),
            source: e,
        })?;
        let mut loaded = LoadedEnv::new();
        let entries = parse_raw_entries(&content, EnvSource::File(p.to_path_buf()))?;
        for var in entries {
            loaded.insert(var);
        }
        loaded.loaded_files.push(p.to_path_buf());
        Ok(loaded)
    }

    /// Default loader discovering `.env` and `.env.local` in current working directory.
    pub fn load_default() -> Result<LoadedEnv, EnvError> {
        Self::new().load()
    }
}

impl Default for EnvLoader {
    fn default() -> Self {
        Self::new()
    }
}

/// Searches up the directory tree to find a directory containing `.env`.
fn find_env_dir(start: &Path) -> Option<PathBuf> {
    let mut current = start.to_path_buf();
    loop {
        if current.join(".env").is_file() || current.join(".env.local").is_file() {
            return Some(current);
        }
        if !current.pop() {
            break;
        }
    }
    None
}

/// Convenience function to load `.env` and `.env.local` from the current directory.
pub fn load_dotenv() -> Result<LoadedEnv, EnvError> {
    EnvLoader::new().load()
}

/// Convenience function to load `.env` and `.env.local` from a specific directory.
pub fn load_dotenv_from(dir: impl AsRef<Path>) -> Result<LoadedEnv, EnvError> {
    EnvLoader::new().directory(dir).load()
}

/// Convenience function to load `.env` with environment-specific overrides (e.g. `.env.development`).
pub fn load_dotenv_with_mode(
    dir: impl AsRef<Path>,
    mode: Option<&str>,
) -> Result<LoadedEnv, EnvError> {
    let mut loader = EnvLoader::new().directory(dir);
    if let Some(m) = mode {
        loader = loader.environment(m);
    }
    loader.load()
}

/// Convenience function to load the full 5-tier configuration hierarchy.
pub fn load_hierarchy() -> Result<LoadedEnv, EnvError> {
    EnvLoader::new().load_hierarchy()
}

/// Convenience function to load the full 5-tier configuration hierarchy from a specific directory with optional CLI overrides.
pub fn load_hierarchy_from(
    dir: impl AsRef<Path>,
    mode: Option<&str>,
    cli_flags: Option<HashMap<String, String>>,
) -> Result<LoadedEnv, EnvError> {
    let mut loader = EnvLoader::new().directory(dir);
    if let Some(m) = mode {
        loader = loader.environment(m);
    }
    if let Some(flags) = cli_flags {
        loader = loader.cli_flags(flags);
    }
    loader.load_hierarchy()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_basic_key_value_parsing() {
        let input = r#"
# Basic config
PORT=8080
HOST=127.0.0.1
DEBUG=true
"#;
        let env = EnvLoader::parse_str(input).unwrap();
        assert_eq!(env.get("PORT"), Some("8080"));
        assert_eq!(env.get("HOST"), Some("127.0.0.1"));
        assert_eq!(env.get_bool("DEBUG"), Some(true));
        assert_eq!(env.get_int::<u16>("PORT"), Some(8080));
    }

    #[test]
    fn test_export_prefix_and_whitespace() {
        let input = r#"
export APP_NAME=fusion
  export   APP_VERSION=0.3.0
KEY_WITH_SPACES = "   spaced value   "
"#;
        let env = EnvLoader::parse_str(input).unwrap();
        assert_eq!(env.get("APP_NAME"), Some("fusion"));
        assert_eq!(env.get("APP_VERSION"), Some("0.3.0"));
        assert_eq!(env.get("KEY_WITH_SPACES"), Some("   spaced value   "));
    }

    #[test]
    fn test_quotes_and_escapes() {
        let input = r#"
SINGLE_QUOTED='hello $USER\nworld'
DOUBLE_QUOTED="hello\nworld\t\"escaped\""
MULTILINE_SINGLE='first line
second line
third line'
MULTILINE_DOUBLE="line 1\nline 2
line 3"
"#;
        let env = EnvLoader::parse_str(input).unwrap();
        // Single quoted preserves literal characters verbatim
        assert_eq!(env.get("SINGLE_QUOTED"), Some("hello $USER\\nworld"));
        // Double quoted handles escape sequences
        assert_eq!(env.get("DOUBLE_QUOTED"), Some("hello\nworld\t\"escaped\""));
        assert_eq!(
            env.get("MULTILINE_SINGLE"),
            Some("first line\nsecond line\nthird line")
        );
        assert_eq!(
            env.get("MULTILINE_DOUBLE"),
            Some("line 1\nline 2\nline 3")
        );
    }

    #[test]
    fn test_inline_comments() {
        let input = r#"
URL=https://example.com/api # trailing comment
URL_WITH_HASH="https://example.com/#section" # another comment
UNQUOTED_HASH=foo#bar # inline comment after space
"#;
        let env = EnvLoader::parse_str(input).unwrap();
        assert_eq!(env.get("URL"), Some("https://example.com/api"));
        assert_eq!(env.get("URL_WITH_HASH"), Some("https://example.com/#section"));
        assert_eq!(env.get("UNQUOTED_HASH"), Some("foo#bar"));
    }

    #[test]
    fn test_variable_expansion_basic() {
        let input = r#"
BASE_DIR=/var/fusion
LOG_DIR=$BASE_DIR/logs
CONFIG_FILE=${BASE_DIR}/config.json
ESCAPED_DOLLAR=$$DOLLAR
"#;
        let env = EnvLoader::parse_str(input).unwrap();
        assert_eq!(env.get("BASE_DIR"), Some("/var/fusion"));
        assert_eq!(env.get("LOG_DIR"), Some("/var/fusion/logs"));
        assert_eq!(env.get("CONFIG_FILE"), Some("/var/fusion/config.json"));
        assert_eq!(env.get("ESCAPED_DOLLAR"), Some("$DOLLAR"));
    }

    #[test]
    fn test_variable_expansion_defaults() {
        let input = r#"
DEFAULT_PORT=${UNSET_PORT:-8080}
EMPTY_VAR=""
FALLBACK_ON_EMPTY=${EMPTY_VAR:-fallback}
FALLBACK_WITHOUT_COLON=${EMPTY_VAR-default}
ASSIGN_DEFAULT=${UNSET_ASSIGN:=created_value}
"#;
        let env = EnvLoader::parse_str(input).unwrap();
        assert_eq!(env.get("DEFAULT_PORT"), Some("8080"));
        assert_eq!(env.get("FALLBACK_ON_EMPTY"), Some("fallback"));
        assert_eq!(env.get("FALLBACK_WITHOUT_COLON"), Some(""));
        assert_eq!(env.get("ASSIGN_DEFAULT"), Some("created_value"));
    }

    #[test]
    fn test_variable_expansion_alternate_and_error() {
        let input = r#"
SET_VAR=active
ALT_VAL=${SET_VAR:+enabled}
UNSET_ALT=${UNSET_VAR:+enabled}
"#;
        let env = EnvLoader::parse_str(input).unwrap();
        assert_eq!(env.get("ALT_VAL"), Some("enabled"));
        assert_eq!(env.get("UNSET_ALT"), Some(""));

        let err_input = "REQUIRED_VAR=${UNSET_SECRET:?API key is mandatory}";
        let res = EnvLoader::parse_str(err_input);
        assert!(res.is_err());
        match res.unwrap_err() {
            EnvError::UndefinedVariable { variable, message, .. } => {
                assert_eq!(variable, "UNSET_SECRET");
                assert_eq!(message, "API key is mandatory");
            }
            other => panic!("Expected UndefinedVariable, got {:?}", other),
        }
    }

    #[test]
    fn test_cyclic_variable_expansion() {
        let input = r#"
A=$B
B=$A
"#;
        let res = EnvLoader::parse_str(input);
        assert!(res.is_err());
        match res.unwrap_err() {
            EnvError::CyclicVariable { variable, .. } => {
                assert!(variable == "A" || variable == "B");
            }
            other => panic!("Expected CyclicVariable error, got {:?}", other),
        }
    }

    #[test]
    fn test_secret_detection_and_masking() {
        let input = r#"
ANTHROPIC_API_KEY=sk-ant-api03-abcdef1234567890abcdef1234567890
DATABASE_URL=postgres://user:super_secret_pw@localhost:5432/fusion
NORMAL_VAR=hello_world
SHORT_SECRET_KEY=12345
"#;
        let env = EnvLoader::parse_str(input).unwrap();
        assert!(env.is_secret("ANTHROPIC_API_KEY"));
        assert!(env.is_secret("DATABASE_URL"));
        assert!(!env.is_secret("NORMAL_VAR"));
        assert!(env.is_secret("SHORT_SECRET_KEY"));

        // Partial mask style
        let masked_anthropic = env.get_masked("ANTHROPIC_API_KEY").unwrap();
        assert!(masked_anthropic.starts_with("sk-ant-"));
        assert!(masked_anthropic.contains("***"));
        assert!(!masked_anthropic.contains("abcdef1234567890abcdef1234567890"));

        let masked_db = env.get_masked("DATABASE_URL").unwrap();
        assert!(!masked_db.contains("super_secret_pw"));
        assert!(masked_db.contains("postgres://user:***@localhost:5432/fusion"));

        let masked_short = env.get_masked("SHORT_SECRET_KEY").unwrap();
        assert_eq!(masked_short, "***");

        // Full mask style
        let full_masked_env = env.clone().with_mask_style(MaskStyle::Full);
        assert_eq!(
            full_masked_env.get_masked("ANTHROPIC_API_KEY").unwrap(),
            "********"
        );

        // TypeOnly mask style
        let type_masked_env = env.clone().with_mask_style(MaskStyle::TypeOnly);
        assert!(type_masked_env
            .get_masked("ANTHROPIC_API_KEY")
            .unwrap()
            .starts_with("[SECRET: length "));

        // Hash mask style
        let hash_masked_env = env.clone().with_mask_style(MaskStyle::Hash);
        assert!(hash_masked_env
            .get_masked("ANTHROPIC_API_KEY")
            .unwrap()
            .starts_with("[SECRET: #"));
    }

    #[test]
    fn test_sanitize_log_text() {
        let input = r#"
OPENAI_API_KEY=sk-abcdefghijklmnopqrstuvwxyz1234567890
"#;
        let env = EnvLoader::parse_str(input).unwrap();
        let log_msg = "Error sending request with Authorization: Bearer sk-abcdefghijklmnopqrstuvwxyz1234567890 to endpoint";
        let sanitized = env.sanitize_text(log_msg);

        assert!(!sanitized.contains("sk-abcdefghijklmnopqrstuvwxyz1234567890"));
        assert!(sanitized.contains("sk-...***...7890"));
    }

    #[test]
    fn test_standalone_sanitize_and_mask_api_key() {
        let key = "sk-ant-api03-abcdef1234567890abcdef1234567890";
        let masked = mask_api_key(key);
        assert!(masked.starts_with("sk-ant-"));
        assert!(masked.contains("***"));
        assert!(!masked.contains("abcdef1234567890abcdef1234567890"));

        let log = "Request failed with Authorization: Bearer sk-ant-api03-abcdef1234567890abcdef1234567890 on host";
        let sanitized = sanitize_text_secrets(log);
        assert!(!sanitized.contains("abcdef1234567890abcdef1234567890"));
    }

    #[test]
    fn test_debug_and_display_formatting_is_safe() {
        let input = r#"
OPENROUTER_API_KEY=sk-or-v1-abcdef12345678901234567890
PUBLIC_NAME=Fusion
"#;
        let env = EnvLoader::parse_str(input).unwrap();
        let debug_str = format!("{:?}", env);
        let display_str = format!("{}", env);

        assert!(!debug_str.contains("sk-or-v1-abcdef12345678901234567890"));
        assert!(!display_str.contains("sk-or-v1-abcdef12345678901234567890"));
        assert!(debug_str.contains("PUBLIC_NAME"));
        assert!(display_str.contains("PUBLIC_NAME=Fusion"));
    }

    #[test]
    fn test_cascading_and_local_overrides() {
        let dir = tempdir().unwrap();
        let dir_path = dir.path();

        // Write .env
        let env_file = dir_path.join(".env");
        std::fs::write(
            &env_file,
            r#"
APP_ENV=base
BASE_ONLY=base_value
OVERRIDDEN_IN_LOCAL=original
OVERRIDDEN_IN_MODE=original
"#,
        )
        .unwrap();

        // Write .env.local
        let env_local_file = dir_path.join(".env.local");
        std::fs::write(
            &env_local_file,
            r#"
OVERRIDDEN_IN_LOCAL=local_override
LOCAL_ONLY=local_value
"#,
        )
        .unwrap();

        // Write .env.development
        let env_dev_file = dir_path.join(".env.development");
        std::fs::write(
            &env_dev_file,
            r#"
APP_ENV=development
OVERRIDDEN_IN_MODE=dev_override
DEV_ONLY=dev_value
"#,
        )
        .unwrap();

        let loaded = EnvLoader::new()
            .directory(dir_path)
            .environment("development")
            .load()
            .unwrap();

        assert_eq!(loaded.get("BASE_ONLY"), Some("base_value"));
        assert_eq!(loaded.get("OVERRIDDEN_IN_LOCAL"), Some("local_override"));
        assert_eq!(loaded.get("LOCAL_ONLY"), Some("local_value"));
        assert_eq!(loaded.get("APP_ENV"), Some("development"));
        assert_eq!(loaded.get("OVERRIDDEN_IN_MODE"), Some("dev_override"));
        assert_eq!(loaded.get("DEV_ONLY"), Some("dev_value"));
    }

    #[test]
    fn test_multi_tier_hierarchy_precedence() {
        let dir = tempdir().unwrap();
        let dir_path = dir.path();

        // Tier 1: Global Config
        let global_dir = dir_path.join("global_config");
        std::fs::create_dir_all(&global_dir).unwrap();
        let global_file = global_dir.join("config.json");
        std::fs::write(
            &global_file,
            r#"{
            "model": "global-model-v1",
            "global_only": "from-global",
            "overridden_by_project": "global-val",
            "overridden_by_env": "global-val",
            "overridden_by_cli": "global-val"
        }"#,
        )
        .unwrap();

        // Tier 2: Project Config
        let project_dir = dir_path.join("project_root");
        let fusion_sub = project_dir.join(".fusion");
        std::fs::create_dir_all(&fusion_sub).unwrap();
        let project_file = fusion_sub.join("config.json");
        std::fs::write(
            &project_file,
            r#"{
            "model": "project-model-v2",
            "project_only": "from-project",
            "overridden_by_project": "project-val",
            "overridden_by_env": "project-val",
            "overridden_by_cli": "project-val"
        }"#,
        )
        .unwrap();

        // Tier 3: Local .env
        let env_file = project_dir.join(".env");
        std::fs::write(
            &env_file,
            r#"
model=dotenv-model-v3
env_only=from-dotenv
overridden_by_env=dotenv-val
overridden_by_cli=dotenv-val
"#,
        )
        .unwrap();

        // Tier 5: CLI Flag
        let loaded = EnvLoader::new()
            .directory(&project_dir)
            .global_config_file(&global_file)
            .project_config_file(&project_file)
            .cli_flag("overridden_by_cli", "cli-val")
            .cli_flag("cli_only", "from-cli")
            .load()
            .unwrap();

        // Verify Tier 1
        assert_eq!(loaded.get("global_only"), Some("from-global"));
        assert_eq!(
            loaded.get_tier("global_only"),
            Some(HierarchyTier::GlobalConfig)
        );

        // Verify Tier 2 overrides Tier 1
        assert_eq!(loaded.get("project_only"), Some("from-project"));
        assert_eq!(
            loaded.get("overridden_by_project"),
            Some("project-val")
        );
        assert_eq!(
            loaded.get_tier("overridden_by_project"),
            Some(HierarchyTier::ProjectConfig)
        );

        // Verify Tier 3 overrides Tier 2 and Tier 1
        assert_eq!(loaded.get("env_only"), Some("from-dotenv"));
        assert_eq!(loaded.get("overridden_by_env"), Some("dotenv-val"));
        assert_eq!(
            loaded.get_tier("overridden_by_env"),
            Some(HierarchyTier::LocalDotEnv)
        );

        // Verify Tier 5 overrides all lower tiers
        assert_eq!(loaded.get("cli_only"), Some("from-cli"));
        assert_eq!(loaded.get("overridden_by_cli"), Some("cli-val"));
        assert_eq!(
            loaded.get_tier("overridden_by_cli"),
            Some(HierarchyTier::CliFlag)
        );

        // Verify model was overridden up to Tier 3 (dotenv)
        assert_eq!(loaded.get("model"), Some("dotenv-model-v3"));
    }

    #[test]
    fn test_json_config_parsing() {
        let json_content = r#"{
            "deepseek_api_key": "sk-deepseek-1234567890abcdef1234567890",
            "model": "deepseek-chat",
            "temperature": 0.7,
            "max_tokens": 4096,
            "stream": true,
            "env": {
                "CUSTOM_ENV_VAR": "custom_value",
                "NUMERIC_VAL": 123
            }
        }"#;

        let entries =
            parse_json_config_str(json_content, EnvSource::GlobalConfig(PathBuf::from("config.json")))
                .unwrap();

        let mut map = HashMap::new();
        for e in entries {
            map.insert(e.key.clone(), e);
        }

        // Check original and uppercase aliases
        assert!(map.contains_key("deepseek_api_key"));
        assert!(map.contains_key("DEEPSEEK_API_KEY"));
        assert!(map.get("DEEPSEEK_API_KEY").unwrap().is_secret);

        assert_eq!(map.get("model").unwrap().value, "deepseek-chat");
        assert_eq!(map.get("MODEL").unwrap().value, "deepseek-chat");
        assert_eq!(map.get("temperature").unwrap().value, "0.7");
        assert_eq!(map.get("max_tokens").unwrap().value, "4096");
        assert_eq!(map.get("stream").unwrap().value, "true");

        // Check nested env object
        assert_eq!(map.get("CUSTOM_ENV_VAR").unwrap().value, "custom_value");
        assert_eq!(map.get("NUMERIC_VAL").unwrap().value, "123");
    }

    #[test]
    fn test_format_table() {
        let input = r#"
PORT=3000
API_KEY=sk-test12345678901234567890
"#;
        let env = EnvLoader::parse_str(input).unwrap();
        let table = env.format_table();
        assert!(table.contains("PORT"));
        assert!(table.contains("API_KEY"));
        assert!(table.contains("YES"));
    }
}
