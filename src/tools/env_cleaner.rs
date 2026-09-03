//! Environment sanitizer and secret scrubber tool for child processes, logs, and outputs.
//!
//! Automatically filters and strips API keys, authentication tokens,
//! cloud credentials, private keys, database passwords, and bearer tokens
//! before spawning external subprocesses or outputting logs, preventing
//! accidental secret leakage in strings, tool outputs, and execution environments.

use async_trait::async_trait;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use crate::tools::types::{Tool, ToolContext};

// ===========================================================================
// Static Sets & Heuristics for Environment Variables
// ===========================================================================

/// Known exact sensitive environment variable names (matched case-insensitively).
pub static KNOWN_SECRET_KEYS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    let mut s = HashSet::new();
    // AI Providers
    s.insert("OPENAI_API_KEY");
    s.insert("OPENAI_ORG_ID");
    s.insert("OPENAI_ORGANIZATION");
    s.insert("OPENAI_PROJECT_ID");
    s.insert("ANTHROPIC_API_KEY");
    s.insert("CLAUDE_API_KEY");
    s.insert("GEMINI_API_KEY");
    s.insert("GOOGLE_API_KEY");
    s.insert("GOOGLE_APPLICATION_CREDENTIALS");
    s.insert("GCP_SERVICE_ACCOUNT_KEY");
    s.insert("GOOGLE_CREDENTIALS");
    s.insert("FIREBASE_TOKEN");
    s.insert("MISTRAL_API_KEY");
    s.insert("GROQ_API_KEY");
    s.insert("COHERE_API_KEY");
    s.insert("COHERE_TOKEN");
    s.insert("TOGETHER_API_KEY");
    s.insert("TOGETHERAI_API_KEY");
    s.insert("FIREWORKS_API_KEY");
    s.insert("DEEPSEEK_API_KEY");
    s.insert("PERPLEXITY_API_KEY");
    s.insert("PPLX_API_KEY");
    s.insert("VOYAGE_API_KEY");
    s.insert("OPENROUTER_API_KEY");
    s.insert("AI21_API_KEY");
    s.insert("REPLICATE_API_TOKEN");
    s.insert("HF_TOKEN");
    s.insert("HUGGING_FACE_HUB_TOKEN");
    s.insert("HUGGINGFACE_TOKEN");
    s.insert("OLLAMA_API_KEY");
    s.insert("XAI_API_KEY");

    // Cloud & Infrastructure
    s.insert("AWS_ACCESS_KEY_ID");
    s.insert("AWS_SECRET_ACCESS_KEY");
    s.insert("AWS_SESSION_TOKEN");
    s.insert("AWS_SECURITY_TOKEN");
    s.insert("AWS_SHARED_CREDENTIALS_FILE");
    s.insert("AZURE_OPENAI_API_KEY");
    s.insert("AZURE_API_KEY");
    s.insert("AZURE_CLIENT_SECRET");
    s.insert("AZURE_TENANT_ID");
    s.insert("AZURE_CLIENT_ID");
    s.insert("AZURE_STORAGE_KEY");
    s.insert("AZURE_STORAGE_ACCOUNT");
    s.insert("AZURE_STORAGE_CONNECTION_STRING");
    s.insert("DIGITALOCEAN_ACCESS_TOKEN");
    s.insert("DO_TOKEN");
    s.insert("LINODE_TOKEN");
    s.insert("VULTR_API_KEY");
    s.insert("CLOUDFLARE_API_TOKEN");
    s.insert("CLOUDFLARE_API_KEY");
    s.insert("CF_API_TOKEN");
    s.insert("CF_API_KEY");
    s.insert("HEROKU_API_KEY");
    s.insert("VERCEL_TOKEN");
    s.insert("NETLIFY_AUTH_TOKEN");
    s.insert("FLY_API_TOKEN");

    // Version Control & Repositories
    s.insert("GITHUB_TOKEN");
    s.insert("GH_TOKEN");
    s.insert("GITHUB_PAT");
    s.insert("GITLAB_TOKEN");
    s.insert("BITBUCKET_TOKEN");
    s.insert("GIT_PASSWORD");
    s.insert("GIT_ASKPASS");

    // Databases
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
    s.insert("COUCHDB_PASSWORD");

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
    s.insert("SQUARE_ACCESS_TOKEN");
    s.insert("BRAINTREE_PRIVATE_KEY");
    s.insert("SENDGRID_API_KEY");
    s.insert("TWILIO_AUTH_TOKEN");
    s.insert("TWILIO_ACCOUNT_SID");
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
    s.insert("ACCESS_KEY");
    s.insert("ACCESS_TOKEN");
    s.insert("REFRESH_TOKEN");
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

/// Safe system environment variable names that must NEVER be stripped by heuristics.
pub static SAFE_SYSTEM_VARIABLES: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    let mut s = HashSet::new();
    // Path & Shell
    s.insert("PATH");
    s.insert("PATHEXT");
    s.insert("SHELL");
    s.insert("COMSPEC");

    // User & Identity
    s.insert("USER");
    s.insert("USERNAME");
    s.insert("LOGNAME");
    s.insert("HOME");
    s.insert("HOMEPATH");
    s.insert("HOMEDRIVE");
    s.insert("USERPROFILE");

    // Terminal & Display
    s.insert("TERM");
    s.insert("TERMINAL_EMULATOR");
    s.insert("COLORTERM");
    s.insert("DISPLAY");
    s.insert("WAYLAND_DISPLAY");
    s.insert("TERM_PROGRAM");
    s.insert("TERM_PROGRAM_VERSION");
    s.insert("WARP_IS_LOCAL_SHELL_SESSION");

    // Locale & Encoding
    s.insert("LANG");
    s.insert("LC_ALL");
    s.insert("LC_CTYPE");
    s.insert("LC_MESSAGES");
    s.insert("LC_COLLATE");
    s.insert("LC_MONETARY");
    s.insert("LC_NUMERIC");
    s.insert("LC_TIME");
    s.insert("TZ");

    // Working Directory & System
    s.insert("PWD");
    s.insert("OLDPWD");
    s.insert("SHLVL");
    s.insert("_");
    s.insert("SYSTEMROOT");
    s.insert("WINDIR");
    s.insert("PROGRAMFILES");
    s.insert("PROGRAMFILES(X86)");
    s.insert("PROGRAMDATA");
    s.insert("COMMONPROGRAMFILES");
    s.insert("APPDATA");
    s.insert("LOCALAPPDATA");
    s.insert("TEMP");
    s.insert("TMP");
    s.insert("TMPDIR");

    // Runtime, Compiler & OS Caches
    s.insert("CARGO_HOME");
    s.insert("RUSTUP_HOME");
    s.insert("RUST_BACKTRACE");
    s.insert("RUST_LOG");
    s.insert("NODE_ENV");
    s.insert("PYTHONPATH");
    s.insert("PYTHONUNBUFFERED");
    s.insert("GOPATH");
    s.insert("GOROOT");
    s.insert("JAVA_HOME");
    s.insert("DOTNET_ROOT");
    s.insert("XDG_CONFIG_HOME");
    s.insert("XDG_DATA_HOME");
    s.insert("XDG_CACHE_HOME");
    s.insert("XDG_RUNTIME_DIR");
    s.insert("XDG_STATE_HOME");

    // Harmless Git Metadata
    s.insert("GIT_AUTHOR_NAME");
    s.insert("GIT_AUTHOR_EMAIL");
    s.insert("GIT_COMMITTER_NAME");
    s.insert("GIT_COMMITTER_EMAIL");

    // Network / Proxy (Non-credential)
    s.insert("HTTP_PROXY");
    s.insert("HTTPS_PROXY");
    s.insert("ALL_PROXY");
    s.insert("NO_PROXY");
    s.insert("SSL_CERT_FILE");
    s.insert("SSL_CERT_DIR");

    s
});

/// Sensitive key suffixes (matched against uppercase variable name).
pub static SENSITIVE_KEY_SUFFIXES: &[&str] = &[
    "_API_KEY",
    "_APIKEY",
    "_SECRET_KEY",
    "_SECRET",
    "_SECRETS",
    "_TOKEN",
    "_TOKENS",
    "_PASSWORD",
    "_PASSWORDS",
    "_PASS",
    "_PWD",
    "_CREDENTIAL",
    "_CREDENTIALS",
    "_AUTH",
    "_AUTH_TOKEN",
    "_PRIVATE_KEY",
    "_PRIVKEY",
    "_SIGNATURE",
    "_BEARER",
];

/// Sensitive key infixes / substrings (matched against uppercase variable name).
pub static SENSITIVE_KEY_INFIXES: &[&str] = &[
    "API_KEY",
    "APIKEY",
    "SECRET_KEY",
    "ACCESS_KEY",
    "PRIVATE_KEY",
    "AUTH_TOKEN",
    "BEARER_TOKEN",
    "PASSWORD",
    "PASSWD",
    "CREDENTIAL",
];

/// Sensitive key prefixes (matched against uppercase variable name).
pub static SENSITIVE_KEY_PREFIXES: &[&str] = &["SECRET_", "PRIVATE_"];

// ===========================================================================
// Secret Kinds, Redaction Patterns, and Placeholders
// ===========================================================================

/// Categorization of detected sensitive credential types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretKind {
    OpenAiApiKey,
    AnthropicApiKey,
    GoogleApiKey,
    GitHubToken,
    GitLabToken,
    AwsAccessKey,
    AwsSecretKey,
    SlackToken,
    StripeKey,
    PrivateKey,
    BearerToken,
    Jwt,
    DatabaseUrl,
    HuggingFaceToken,
    NpmToken,
    PyPiToken,
    SendGridApiKey,
    TwilioKey,
    Password,
    GenericApiKey,
    Custom,
}

impl SecretKind {
    /// Returns a human-friendly name for this secret type.
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::OpenAiApiKey => "OpenAI API Key",
            Self::AnthropicApiKey => "Anthropic API Key",
            Self::GoogleApiKey => "Google API Key",
            Self::GitHubToken => "GitHub Token",
            Self::GitLabToken => "GitLab Token",
            Self::AwsAccessKey => "AWS Access Key",
            Self::AwsSecretKey => "AWS Secret Access Key",
            Self::SlackToken => "Slack Token",
            Self::StripeKey => "Stripe Key",
            Self::PrivateKey => "Private Key",
            Self::BearerToken => "Bearer Token",
            Self::Jwt => "JSON Web Token",
            Self::DatabaseUrl => "Database URL Password",
            Self::HuggingFaceToken => "Hugging Face Token",
            Self::NpmToken => "NPM Token",
            Self::PyPiToken => "PyPI Token",
            Self::SendGridApiKey => "SendGrid API Key",
            Self::TwilioKey => "Twilio Key",
            Self::Password => "Password Assignment",
            Self::GenericApiKey => "Generic API Key / Token",
            Self::Custom => "Custom Secret",
        }
    }

    /// Standard type-specific redaction placeholder (e.g. `[REDACTED_API_KEY]`).
    pub fn type_placeholder(&self) -> &'static str {
        match self {
            Self::OpenAiApiKey => "[REDACTED_OPENAI_API_KEY]",
            Self::AnthropicApiKey => "[REDACTED_ANTHROPIC_API_KEY]",
            Self::GoogleApiKey => "[REDACTED_GOOGLE_API_KEY]",
            Self::GitHubToken => "[REDACTED_GITHUB_TOKEN]",
            Self::GitLabToken => "[REDACTED_GITLAB_TOKEN]",
            Self::AwsAccessKey => "[REDACTED_AWS_ACCESS_KEY]",
            Self::AwsSecretKey => "[REDACTED_AWS_SECRET_KEY]",
            Self::SlackToken => "[REDACTED_SLACK_TOKEN]",
            Self::StripeKey => "[REDACTED_STRIPE_KEY]",
            Self::PrivateKey => "[REDACTED_PRIVATE_KEY]",
            Self::BearerToken => "[REDACTED_BEARER_TOKEN]",
            Self::Jwt => "[REDACTED_JWT_TOKEN]",
            Self::DatabaseUrl => "[REDACTED_PASSWORD]",
            Self::HuggingFaceToken => "[REDACTED_HF_TOKEN]",
            Self::NpmToken => "[REDACTED_NPM_TOKEN]",
            Self::PyPiToken => "[REDACTED_PYPI_TOKEN]",
            Self::SendGridApiKey => "[REDACTED_SENDGRID_KEY]",
            Self::TwilioKey => "[REDACTED_TWILIO_KEY]",
            Self::Password => "[REDACTED_PASSWORD]",
            Self::GenericApiKey => "[REDACTED_API_KEY]",
            Self::Custom => "[REDACTED_SECRET]",
        }
    }

    /// Generic category placeholder (e.g. `[REDACTED_API_KEY]`, `[REDACTED_TOKEN]`).
    pub fn generic_placeholder(&self) -> &'static str {
        match self {
            Self::OpenAiApiKey
            | Self::AnthropicApiKey
            | Self::GoogleApiKey
            | Self::SendGridApiKey
            | Self::GenericApiKey => "[REDACTED_API_KEY]",
            Self::GitHubToken
            | Self::GitLabToken
            | Self::SlackToken
            | Self::BearerToken
            | Self::Jwt
            | Self::HuggingFaceToken
            | Self::NpmToken
            | Self::PyPiToken => "[REDACTED_TOKEN]",
            Self::AwsAccessKey | Self::AwsSecretKey | Self::StripeKey | Self::TwilioKey => {
                "[REDACTED_CREDENTIAL]"
            }
            Self::PrivateKey => "[REDACTED_PRIVATE_KEY]",
            Self::DatabaseUrl | Self::Password => "[REDACTED_PASSWORD]",
            Self::Custom => "[REDACTED_SECRET]",
        }
    }
}

/// Style for redaction placeholders when scrubbing sensitive strings.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaceholderStyle {
    /// Type-specific placeholder like `[REDACTED_OPENAI_API_KEY]`, `[REDACTED_GITHUB_TOKEN]`.
    #[default]
    TypeSpecific,
    /// Simple universal placeholder `[REDACTED]`.
    Simple,
    /// Generic category placeholder like `[REDACTED_API_KEY]`, `[REDACTED_TOKEN]`, `[REDACTED_PASSWORD]`.
    Generic,
    /// Safe masked prefix/suffix like `sk-p...3456`.
    Masked,
    /// Custom static placeholder string.
    Custom(String),
}

/// Regex patterns matching sensitive values regardless of variable name.
struct ValuePattern {
    name: &'static str,
    regex: Regex,
}

static SENSITIVE_VALUE_PATTERNS: LazyLock<Vec<ValuePattern>> = LazyLock::new(|| {
    vec![
        ValuePattern {
            name: "Anthropic API Key",
            regex: Regex::new(r"\bsk-ant-[A-Za-z0-9_-]{20,}\b").expect("valid regex"),
        },
        ValuePattern {
            name: "OpenAI API Key",
            regex: Regex::new(r"\bsk-(?:proj-|admin-|org-)?[A-Za-z0-9_-]{20,}\b")
                .expect("valid regex"),
        },
        ValuePattern {
            name: "GitHub Token",
            regex: Regex::new(r"\b(?:gh[pousr]_[A-Za-z0-9_]{36,}|github_pat_[A-Za-z0-9_]{50,})\b")
                .expect("valid regex"),
        },
        ValuePattern {
            name: "GitLab Token",
            regex: Regex::new(r"\bglpat-[0-9a-zA-Z_\-]{20,}\b").expect("valid regex"),
        },
        ValuePattern {
            name: "AWS Access Key",
            regex: Regex::new(r"\b(?:AKIA|ASIA|AROA)[A-Z0-9]{16}\b").expect("valid regex"),
        },
        ValuePattern {
            name: "Google API Key",
            regex: Regex::new(r"\bAIza[0-9A-Za-z_-]{30,45}\b").expect("valid regex"),
        },
        ValuePattern {
            name: "Slack Token",
            regex: Regex::new(r"\bxox[baprs]-[0-9a-zA-Z-]{10,}\b").expect("valid regex"),
        },
        ValuePattern {
            name: "Stripe Secret Key",
            regex: Regex::new(r"\b[sr]k_(?:test|live)_[0-9a-zA-Z]{24,}\b").expect("valid regex"),
        },
        ValuePattern {
            name: "Private Key Header",
            regex: Regex::new(r"-----BEGIN (?:[A-Z0-9_ ]+ )?PRIVATE KEY-----")
                .expect("valid regex"),
        },
        ValuePattern {
            name: "Bearer Token",
            regex: Regex::new(r"(?i)^bearer\s+[A-Za-z0-9\-._~+/]+=*$").expect("valid regex"),
        },
        ValuePattern {
            name: "JSON Web Token",
            regex: Regex::new(r"\beyJ[A-Za-z0-9_-]{8,}\.eyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\b")
                .expect("valid regex"),
        },
        ValuePattern {
            name: "Hugging Face Token",
            regex: Regex::new(r"\bhf_[a-zA-Z0-9]{30,}\b").expect("valid regex"),
        },
        ValuePattern {
            name: "NPM Token",
            regex: Regex::new(r"\bnpm_[a-zA-Z0-9]{30,}\b").expect("valid regex"),
        },
        ValuePattern {
            name: "PyPI Token",
            regex: Regex::new(r"\bpypi-AgEIcHlwaS5vcmc[A-Za-z0-9_-]{30,}\b").expect("valid regex"),
        },
        ValuePattern {
            name: "SendGrid API Key",
            regex: Regex::new(r"\bSG\.[A-Za-z0-9_-]{22}\.[A-Za-z0-9_-]{43}\b")
                .expect("valid regex"),
        },
        ValuePattern {
            name: "Twilio Key",
            regex: Regex::new(r"\bSK[0-9a-fA-F]{32}\b").expect("valid regex"),
        },
    ]
});

/// A finding produced when scanning or redacting text for sensitive secrets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretFinding {
    /// The classified kind of secret.
    pub kind: SecretKind,
    /// Descriptive name of the rule that matched.
    pub rule_name: String,
    /// 0-indexed start byte offset in the input text.
    pub start: usize,
    /// 0-indexed end byte offset in the input text.
    pub end: usize,
    /// 1-indexed line number where the match begins.
    pub line: usize,
    /// Safe masked representation of the matched secret.
    pub masked_sample: String,
}

/// Type alias for `SecretFinding` for consistency across modules.
pub type RedactionFinding = SecretFinding;

/// Custom rule for user-defined redactions.
#[derive(Debug, Clone)]
pub struct CustomRedactionRule {
    pub name: String,
    pub pattern: Regex,
    pub placeholder: String,
}

// ===========================================================================
// Secret Scrubber & Redaction Engine
// ===========================================================================

/// Engine for scanning, detecting, and redacting credentials and secrets in text,
/// tool outputs, error logs, and JSON structures.
#[derive(Debug, Clone)]
pub struct SecretScrubber {
    placeholder_style: PlaceholderStyle,
    custom_rules: Vec<CustomRedactionRule>,
    known_secrets: HashSet<String>,
}

impl Default for SecretScrubber {
    fn default() -> Self {
        Self::new()
    }
}

// Pre-compiled regexes for text redaction
static PRIV_KEY_BLOCK_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"-----BEGIN (?:[A-Z0-9_ ]+ )?PRIVATE KEY-----[\s\S]*?-----END (?:[A-Z0-9_ ]+ )?PRIVATE KEY-----")
        .expect("valid regex")
});

static PRIV_KEY_HEADER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"-----BEGIN (?:[A-Z0-9_ ]+ )?PRIVATE KEY-----.*").expect("valid regex")
});

static DB_URI_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b((?:postgres|postgresql|mysql|mongodb|mongodb\+srv|redis|rediss|amqp|amqps)://(?:[^:\s/]*:)?)([^@\s/]+)(@[a-zA-Z0-9.\-_]+(?::[0-9]+)?(?:/[^\s]*)?)\b")
        .expect("valid regex")
});

static BEARER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\bBearer\s+([A-Za-z0-9\-._~+/]+=*)").expect("valid regex"));

static JWT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\beyJ[A-Za-z0-9_-]{8,}\.eyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\b")
        .expect("valid regex")
});

static ANTHROPIC_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bsk-ant-[A-Za-z0-9_-]{20,}\b").expect("valid regex"));

static OPENAI_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\bsk-(?:proj-|admin-|org-)?[A-Za-z0-9_-]{20,}\b").expect("valid regex")
});

static GOOGLE_KEY_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bAIza[0-9A-Za-z_-]{30,45}\b").expect("valid regex"));

static GITHUB_TOKEN_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?:gh[pousr]_[A-Za-z0-9_]{36,}|github_pat_[A-Za-z0-9_]{50,})\b")
        .expect("valid regex")
});

static GITLAB_TOKEN_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bglpat-[0-9a-zA-Z_\-]{20,}\b").expect("valid regex"));

static AWS_KEY_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(?:AKIA|ASIA|AROA)[A-Z0-9]{16}\b").expect("valid regex"));

static AWS_SECRET_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)\b(aws_secret_access_key|aws_secret_key)\s*([:=])\s*['"]?([A-Za-z0-9/+=]{40})['"]?"#,
    )
    .expect("valid regex")
});

static SLACK_TOKEN_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bxox[baprs]-[0-9a-zA-Z-]{10,}\b").expect("valid regex"));

static STRIPE_KEY_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b[sr]k_(?:test|live)_[0-9a-zA-Z]{24,}\b").expect("valid regex"));

static HF_TOKEN_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bhf_[a-zA-Z0-9]{30,}\b").expect("valid regex"));

static NPM_TOKEN_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bnpm_[a-zA-Z0-9]{30,}\b").expect("valid regex"));

static PYPI_TOKEN_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\bpypi-AgEIcHlwaS5vcmc[A-Za-z0-9_-]{30,}\b").expect("valid regex")
});

static SENDGRID_KEY_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\bSG\.[A-Za-z0-9_-]{22}\.[A-Za-z0-9_-]{43}\b").expect("valid regex")
});

static TWILIO_KEY_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bSK[0-9a-fA-F]{32}\b").expect("valid regex"));

static PASSWORD_ASSIGN_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)\b(password|passwd|pwd|db_pass|secret_key|client_secret)\s*([:=])\s*['"]?([^'"\s\n,;]{4,})['"]?"#)
        .expect("valid regex")
});

static QUERY_PARAM_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)([?&](?:api_key|apikey|token|access_token|secret|auth)=)([^&\s]+)"#)
        .expect("valid regex")
});

impl SecretScrubber {
    /// Creates a new `SecretScrubber` with standard redaction rules and default placeholder style.
    pub fn new() -> Self {
        Self {
            placeholder_style: PlaceholderStyle::TypeSpecific,
            custom_rules: Vec::new(),
            known_secrets: HashSet::new(),
        }
    }

    /// Sets the placeholder style for redacted secrets.
    pub fn with_placeholder_style(mut self, style: PlaceholderStyle) -> Self {
        self.placeholder_style = style;
        self
    }

    /// Adds a custom redaction rule with a regex pattern.
    pub fn with_custom_rule(
        mut self,
        name: impl Into<String>,
        pattern: Regex,
        placeholder: impl Into<String>,
    ) -> Self {
        self.custom_rules.push(CustomRedactionRule {
            name: name.into(),
            pattern,
            placeholder: placeholder.into(),
        });
        self
    }

    /// Adds a specific known secret string to always redact.
    pub fn with_known_secret(mut self, secret: impl Into<String>) -> Self {
        let s = secret.into();
        if !s.trim().is_empty() {
            self.known_secrets.insert(s);
        }
        self
    }

    /// Adds multiple known secret strings to redact.
    pub fn with_known_secrets<I, S>(mut self, secrets: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for s in secrets {
            let val = s.into();
            if !val.trim().is_empty() {
                self.known_secrets.insert(val);
            }
        }
        self
    }

    /// Returns the active placeholder style.
    pub fn placeholder_style(&self) -> &PlaceholderStyle {
        &self.placeholder_style
    }

    /// Resolves the placeholder string for a given secret kind and matched text.
    fn resolve_placeholder(&self, kind: SecretKind, original: &str) -> String {
        match &self.placeholder_style {
            PlaceholderStyle::TypeSpecific => kind.type_placeholder().to_string(),
            PlaceholderStyle::Simple => "[REDACTED]".to_string(),
            PlaceholderStyle::Generic => kind.generic_placeholder().to_string(),
            PlaceholderStyle::Masked => mask_value(original),
            PlaceholderStyle::Custom(c) => c.clone(),
        }
    }

    /// Scans text and returns all identified secret findings.
    pub fn find_secrets(&self, text: &str) -> Vec<SecretFinding> {
        let mut findings = Vec::new();

        let line_starts: Vec<usize> = std::iter::once(0)
            .chain(
                text.match_indices('\n')
                    .map(|(idx, _)| idx.saturating_add(1)),
            )
            .collect();

        let get_line = |byte_offset: usize| -> usize {
            match line_starts.binary_search(&byte_offset) {
                Ok(line_idx) => line_idx + 1,
                Err(line_idx) => line_idx,
            }
        };

        // Scan known exact secrets
        for secret in &self.known_secrets {
            for (start, matched) in text.match_indices(secret) {
                findings.push(SecretFinding {
                    kind: SecretKind::Custom,
                    rule_name: "Known Secret".to_string(),
                    start,
                    end: start + matched.len(),
                    line: get_line(start),
                    masked_sample: mask_value(matched),
                });
            }
        }

        // Helper macro/closure for regex finding
        let mut check_rule = |kind: SecretKind, rule_name: &'static str, re: &Regex| {
            for m in re.find_iter(text) {
                findings.push(SecretFinding {
                    kind,
                    rule_name: rule_name.to_string(),
                    start: m.start(),
                    end: m.end(),
                    line: get_line(m.start()),
                    masked_sample: mask_value(m.as_str()),
                });
            }
        };

        check_rule(
            SecretKind::PrivateKey,
            "Private Key Block",
            &PRIV_KEY_BLOCK_RE,
        );
        check_rule(
            SecretKind::AnthropicApiKey,
            "Anthropic API Key",
            &ANTHROPIC_RE,
        );
        check_rule(SecretKind::OpenAiApiKey, "OpenAI API Key", &OPENAI_RE);
        check_rule(SecretKind::GoogleApiKey, "Google API Key", &GOOGLE_KEY_RE);
        check_rule(SecretKind::GitHubToken, "GitHub Token", &GITHUB_TOKEN_RE);
        check_rule(SecretKind::GitLabToken, "GitLab Token", &GITLAB_TOKEN_RE);
        check_rule(SecretKind::AwsAccessKey, "AWS Access Key", &AWS_KEY_RE);
        check_rule(SecretKind::SlackToken, "Slack Token", &SLACK_TOKEN_RE);
        check_rule(SecretKind::StripeKey, "Stripe Secret Key", &STRIPE_KEY_RE);
        check_rule(SecretKind::BearerToken, "Bearer Token", &BEARER_RE);
        check_rule(SecretKind::Jwt, "JSON Web Token", &JWT_RE);
        check_rule(SecretKind::DatabaseUrl, "Database URI Password", &DB_URI_RE);
        check_rule(
            SecretKind::HuggingFaceToken,
            "Hugging Face Token",
            &HF_TOKEN_RE,
        );
        check_rule(SecretKind::NpmToken, "NPM Token", &NPM_TOKEN_RE);
        check_rule(SecretKind::PyPiToken, "PyPI Token", &PYPI_TOKEN_RE);
        check_rule(
            SecretKind::SendGridApiKey,
            "SendGrid API Key",
            &SENDGRID_KEY_RE,
        );
        check_rule(SecretKind::TwilioKey, "Twilio Key", &TWILIO_KEY_RE);
        check_rule(
            SecretKind::Password,
            "Password Assignment",
            &PASSWORD_ASSIGN_RE,
        );

        // Custom user rules
        for custom in &self.custom_rules {
            for m in custom.pattern.find_iter(text) {
                findings.push(SecretFinding {
                    kind: SecretKind::Custom,
                    rule_name: custom.name.clone(),
                    start: m.start(),
                    end: m.end(),
                    line: get_line(m.start()),
                    masked_sample: mask_value(m.as_str()),
                });
            }
        }

        // Sort by start position
        findings.sort_by_key(|f| f.start);
        findings
    }

    /// Redacts all sensitive values in the input string and returns the sanitized text.
    pub fn redact_text(&self, text: &str) -> String {
        let (sanitized, _) = self.redact_text_with_findings(text);
        sanitized
    }

    /// Redacts all sensitive values in the input string and returns both the sanitized text
    /// and the list of findings.
    pub fn redact_text_with_findings(&self, text: &str) -> (String, Vec<SecretFinding>) {
        let findings = self.find_secrets(text);

        let mut output = text.to_string();

        // 1. Redact known exact secrets first (longest matches first)
        let mut sorted_secrets: Vec<&String> = self.known_secrets.iter().collect();
        sorted_secrets.sort_by_key(|b| std::cmp::Reverse(b.len()));
        for secret in sorted_secrets {
            let placeholder = self.resolve_placeholder(SecretKind::Custom, secret);
            output = output.replace(secret.as_str(), &placeholder);
        }

        // 2. Redact custom rules
        for custom in &self.custom_rules {
            output = custom
                .pattern
                .replace_all(&output, custom.placeholder.as_str())
                .to_string();
        }

        // 3. Redact private keys (blocks and single-line headers)
        let pk_placeholder = self.resolve_placeholder(SecretKind::PrivateKey, "PRIVATE KEY");
        output = PRIV_KEY_BLOCK_RE
            .replace_all(&output, pk_placeholder.as_str())
            .to_string();
        output = PRIV_KEY_HEADER_RE
            .replace_all(&output, pk_placeholder.as_str())
            .to_string();

        // 4. Redact database URLs with passwords
        let db_placeholder = self.resolve_placeholder(SecretKind::DatabaseUrl, "password");
        output = DB_URI_RE
            .replace_all(&output, |caps: &regex::Captures| {
                format!("{}{}{}", &caps[1], db_placeholder, &caps[3])
            })
            .to_string();

        // 5. Redact Bearer tokens
        let bearer_placeholder = self.resolve_placeholder(SecretKind::BearerToken, "token");
        output = BEARER_RE
            .replace_all(&output, |caps: &regex::Captures| {
                let full = caps.get(0).map_or("Bearer", |m| m.as_str());
                if full.starts_with("bearer") {
                    format!("bearer {bearer_placeholder}")
                } else {
                    format!("Bearer {bearer_placeholder}")
                }
            })
            .to_string();

        // 6. Redact JWTs
        let jwt_placeholder = self.resolve_placeholder(SecretKind::Jwt, "jwt");
        output = JWT_RE
            .replace_all(&output, jwt_placeholder.as_str())
            .to_string();

        // 7. Redact AI Provider Keys (Anthropic before OpenAI so `sk-ant-` gets specific tag)
        let anthropic_placeholder =
            self.resolve_placeholder(SecretKind::AnthropicApiKey, "sk-ant-");
        output = ANTHROPIC_RE
            .replace_all(&output, anthropic_placeholder.as_str())
            .to_string();

        let openai_placeholder = self.resolve_placeholder(SecretKind::OpenAiApiKey, "sk-");
        output = OPENAI_RE
            .replace_all(&output, openai_placeholder.as_str())
            .to_string();

        let google_placeholder = self.resolve_placeholder(SecretKind::GoogleApiKey, "AIza");
        output = GOOGLE_KEY_RE
            .replace_all(&output, google_placeholder.as_str())
            .to_string();

        // 8. Redact VCS & Cloud Tokens
        let gh_placeholder = self.resolve_placeholder(SecretKind::GitHubToken, "ghp_");
        output = GITHUB_TOKEN_RE
            .replace_all(&output, gh_placeholder.as_str())
            .to_string();

        let gitlab_placeholder = self.resolve_placeholder(SecretKind::GitLabToken, "glpat-");
        output = GITLAB_TOKEN_RE
            .replace_all(&output, gitlab_placeholder.as_str())
            .to_string();

        let aws_sec_placeholder = self.resolve_placeholder(SecretKind::AwsSecretKey, "secret");
        output = AWS_SECRET_RE
            .replace_all(&output, |caps: &regex::Captures| {
                format!(
                    "{}{}{}{}{}",
                    &caps[1], &caps[2], &caps[3], aws_sec_placeholder, &caps[3]
                )
            })
            .to_string();

        let aws_key_placeholder = self.resolve_placeholder(SecretKind::AwsAccessKey, "AKIA");
        output = AWS_KEY_RE
            .replace_all(&output, aws_key_placeholder.as_str())
            .to_string();

        let slack_placeholder = self.resolve_placeholder(SecretKind::SlackToken, "xoxb-");
        output = SLACK_TOKEN_RE
            .replace_all(&output, slack_placeholder.as_str())
            .to_string();

        let stripe_placeholder = self.resolve_placeholder(SecretKind::StripeKey, "sk_live_");
        output = STRIPE_KEY_RE
            .replace_all(&output, stripe_placeholder.as_str())
            .to_string();

        let hf_placeholder = self.resolve_placeholder(SecretKind::HuggingFaceToken, "hf_");
        output = HF_TOKEN_RE
            .replace_all(&output, hf_placeholder.as_str())
            .to_string();

        let npm_placeholder = self.resolve_placeholder(SecretKind::NpmToken, "npm_");
        output = NPM_TOKEN_RE
            .replace_all(&output, npm_placeholder.as_str())
            .to_string();

        let pypi_placeholder = self.resolve_placeholder(SecretKind::PyPiToken, "pypi-");
        output = PYPI_TOKEN_RE
            .replace_all(&output, pypi_placeholder.as_str())
            .to_string();

        let sg_placeholder = self.resolve_placeholder(SecretKind::SendGridApiKey, "SG.");
        output = SENDGRID_KEY_RE
            .replace_all(&output, sg_placeholder.as_str())
            .to_string();

        let twilio_placeholder = self.resolve_placeholder(SecretKind::TwilioKey, "SK");
        output = TWILIO_KEY_RE
            .replace_all(&output, twilio_placeholder.as_str())
            .to_string();

        // 9. Redact generic password assignments (e.g. `password = "secret"`)
        let pwd_placeholder = self.resolve_placeholder(SecretKind::Password, "password");
        output = PASSWORD_ASSIGN_RE
            .replace_all(&output, |caps: &regex::Captures| {
                format!(
                    "{}{}{}{}{}",
                    &caps[1], &caps[2], &caps[3], pwd_placeholder, &caps[3]
                )
            })
            .to_string();

        // 10. Redact query parameters in URLs (e.g. `?api_key=secret`)
        let query_placeholder = self.resolve_placeholder(SecretKind::GenericApiKey, "secret");
        output = QUERY_PARAM_RE
            .replace_all(&output, |caps: &regex::Captures| {
                format!("{}{}", &caps[1], query_placeholder)
            })
            .to_string();

        (output, findings)
    }

    /// Recursively redacts sensitive values within a `serde_json::Value`.
    pub fn redact_json(&self, val: &Value) -> Value {
        match val {
            Value::String(s) => Value::String(self.redact_text(s)),
            Value::Array(arr) => Value::Array(arr.iter().map(|v| self.redact_json(v)).collect()),
            Value::Object(map) => {
                let mut clean_map = serde_json::Map::with_capacity(map.len());
                for (k, v) in map {
                    let upper = k.to_uppercase();
                    let is_sensitive = KNOWN_SECRET_KEYS.contains(upper.as_str())
                        || SENSITIVE_KEY_SUFFIXES.iter().any(|&s| upper.ends_with(s))
                        || SENSITIVE_KEY_INFIXES.iter().any(|&i| upper.contains(i))
                        || SENSITIVE_KEY_PREFIXES.iter().any(|&p| upper.starts_with(p));

                    if is_sensitive {
                        match v {
                            Value::String(s) => {
                                clean_map.insert(
                                    k.clone(),
                                    Value::String(
                                        self.resolve_placeholder(SecretKind::GenericApiKey, s),
                                    ),
                                );
                            }
                            _ => {
                                clean_map
                                    .insert(k.clone(), Value::String("[REDACTED]".to_string()));
                            }
                        }
                    } else {
                        clean_map.insert(k.clone(), self.redact_json(v));
                    }
                }
                Value::Object(clean_map)
            }
            other => other.clone(),
        }
    }

    /// Redacts lines of text in a vector.
    pub fn redact_lines(&self, lines: &[&str]) -> Vec<String> {
        lines.iter().map(|l| self.redact_text(l)).collect()
    }

    /// Redacts sensitive values in an environment map.
    pub fn redact_env_map(&self, env: &HashMap<String, String>) -> HashMap<String, String> {
        let mut clean = HashMap::with_capacity(env.len());
        for (k, v) in env {
            clean.insert(k.clone(), self.redact_text(v));
        }
        clean
    }
}

/// Type alias for `SecretScrubber` matching alternative naming conventions.
pub type SecretRedactor = SecretScrubber;

// ===========================================================================
// Environment Sanitization Policy, Reasons, and Results
// ===========================================================================

/// Policy mode for environment sanitization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SanitizationPolicy {
    /// Default: Block known secret keys and patterns, allow everything else safe.
    #[default]
    Blocklist,
    /// Strict: Only allow explicitly allowlisted variables (safe system vars + custom allowed).
    Allowlist,
}

/// The reason why an environment variable was sanitized/stripped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SanitizationReason {
    /// Matched a known exact secret key name.
    KnownSecretKey,
    /// Key name matched a sensitive heuristic pattern (suffix, infix, or prefix).
    SensitiveKeyPattern(String),
    /// Value matched a recognized credential or API key signature.
    SensitiveValuePattern(String),
    /// Strict policy active and key is not in allowlist.
    NotInAllowlist,
    /// User or configuration explicitly blocked this key.
    CustomBlocked,
}

/// Audit report entry describing a stripped environment variable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SanitizationReport {
    /// The environment variable key that was stripped.
    pub key: String,
    /// Reason why the variable was stripped.
    pub reason: SanitizationReason,
    /// Redacted mask of the stripped value (never reveals full secret).
    pub masked_value: String,
}

/// Result of environment sanitization containing the clean map and audit report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SanitizationResult {
    /// Clean environment variables safe to pass to child processes.
    pub clean_env: HashMap<String, String>,
    /// List of stripped variables with reasons.
    pub stripped: Vec<SanitizationReport>,
}

impl SanitizationResult {
    /// Returns true if no variables were stripped.
    pub fn is_clean(&self) -> bool {
        self.stripped.is_empty()
    }

    /// Returns the number of stripped variables.
    pub fn stripped_count(&self) -> usize {
        self.stripped.len()
    }

    /// Returns true if the specified key was stripped (case-insensitive).
    pub fn has_stripped(&self, key: &str) -> bool {
        self.stripped
            .iter()
            .any(|r| r.key.eq_ignore_ascii_case(key))
    }
}

// ===========================================================================
// EnvCleaner Struct & Methods
// ===========================================================================

/// Environment variable sanitizer and credential cleaner.
#[derive(Debug, Clone)]
pub struct EnvCleaner {
    policy: SanitizationPolicy,
    custom_blocked_keys: HashSet<String>,
    custom_allowed_keys: HashSet<String>,
    custom_blocked_patterns: Vec<Regex>,
    scan_values: bool,
    scrubber: SecretScrubber,
}

/// Type alias for `EnvCleaner` to match alternative naming conventions.
pub type EnvSanitizer = EnvCleaner;

impl Default for EnvCleaner {
    fn default() -> Self {
        Self::new()
    }
}

impl EnvCleaner {
    /// Creates a new default `EnvCleaner` using the `Blocklist` policy and value scanning enabled.
    pub fn new() -> Self {
        Self {
            policy: SanitizationPolicy::Blocklist,
            custom_blocked_keys: HashSet::new(),
            custom_allowed_keys: HashSet::new(),
            custom_blocked_patterns: Vec::new(),
            scan_values: true,
            scrubber: SecretScrubber::new(),
        }
    }

    /// Creates a strict allowlist-only `EnvCleaner`.
    pub fn strict() -> Self {
        Self {
            policy: SanitizationPolicy::Allowlist,
            custom_blocked_keys: HashSet::new(),
            custom_allowed_keys: HashSet::new(),
            custom_blocked_patterns: Vec::new(),
            scan_values: true,
            scrubber: SecretScrubber::new(),
        }
    }

    /// Sets the sanitization policy mode.
    pub fn with_policy(mut self, policy: SanitizationPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// Adds a custom key to block.
    pub fn with_blocked_key(mut self, key: impl Into<String>) -> Self {
        self.custom_blocked_keys.insert(key.into().to_uppercase());
        self
    }

    /// Adds multiple custom keys to block.
    pub fn with_blocked_keys<I, S>(mut self, keys: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for k in keys {
            self.custom_blocked_keys.insert(k.into().to_uppercase());
        }
        self
    }

    /// Adds an explicit allowlist override for a key.
    pub fn with_allowed_key(mut self, key: impl Into<String>) -> Self {
        self.custom_allowed_keys.insert(key.into().to_uppercase());
        self
    }

    /// Adds multiple explicit allowlist overrides.
    pub fn with_allowed_keys<I, S>(mut self, keys: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for k in keys {
            self.custom_allowed_keys.insert(k.into().to_uppercase());
        }
        self
    }

    /// Configures whether to scan variable values for credential patterns.
    pub fn with_scan_values(mut self, scan: bool) -> Self {
        self.scan_values = scan;
        self
    }

    /// Adds a custom regex pattern to block keys matching it.
    pub fn with_blocked_pattern(mut self, pattern: Regex) -> Self {
        self.custom_blocked_patterns.push(pattern);
        self
    }

    /// Sets a custom `SecretScrubber` for text and log redactions.
    pub fn with_scrubber(mut self, scrubber: SecretScrubber) -> Self {
        self.scrubber = scrubber;
        self
    }

    /// Adds a known secret string to the inner scrubber.
    pub fn with_known_secret(mut self, secret: impl Into<String>) -> Self {
        self.scrubber = self.scrubber.with_known_secret(secret);
        self
    }

    /// Configures the placeholder style of the inner scrubber.
    pub fn with_placeholder_style(mut self, style: PlaceholderStyle) -> Self {
        self.scrubber = self.scrubber.with_placeholder_style(style);
        self
    }

    /// Returns a reference to the inner `SecretScrubber`.
    pub fn scrubber(&self) -> &SecretScrubber {
        &self.scrubber
    }

    /// Checks whether an environment variable key is considered a safe system variable.
    pub fn is_safe_system_var(key: &str) -> bool {
        let upper = key.to_uppercase();
        SAFE_SYSTEM_VARIABLES.contains(upper.as_str())
    }

    /// Checks if a key name matches any known or heuristic secret pattern.
    pub fn check_key_sensitivity(&self, key: &str) -> Option<SanitizationReason> {
        let upper = key.to_uppercase();

        // Check custom blocked keys first
        if self.custom_blocked_keys.contains(&upper) {
            return Some(SanitizationReason::CustomBlocked);
        }

        // Custom blocked regex patterns
        for pattern in &self.custom_blocked_patterns {
            if pattern.is_match(key) {
                return Some(SanitizationReason::CustomBlocked);
            }
        }

        // Check custom allowed keys (explicit override)
        if self.custom_allowed_keys.contains(&upper) {
            return None;
        }

        // Check known exact secret keys
        if KNOWN_SECRET_KEYS.contains(upper.as_str()) {
            return Some(SanitizationReason::KnownSecretKey);
        }

        // Check if key is a safe system variable - safe vars never match heuristic suffixes/infixes
        if SAFE_SYSTEM_VARIABLES.contains(upper.as_str()) {
            return None;
        }

        // Check suffixes
        for &suffix in SENSITIVE_KEY_SUFFIXES {
            if upper.ends_with(suffix) {
                return Some(SanitizationReason::SensitiveKeyPattern(format!(
                    "suffix {suffix}"
                )));
            }
        }

        // Check infixes
        for &infix in SENSITIVE_KEY_INFIXES {
            if upper.contains(infix) {
                return Some(SanitizationReason::SensitiveKeyPattern(format!(
                    "contains {infix}"
                )));
            }
        }

        // Check prefixes
        for &prefix in SENSITIVE_KEY_PREFIXES {
            if upper.starts_with(prefix) {
                return Some(SanitizationReason::SensitiveKeyPattern(format!(
                    "prefix {prefix}"
                )));
            }
        }

        None
    }

    /// Checks if a variable value matches known credential signatures.
    pub fn check_value_sensitivity(&self, val: &str) -> Option<SanitizationReason> {
        if !self.scan_values || val.trim().is_empty() {
            return None;
        }

        for pat in SENSITIVE_VALUE_PATTERNS.iter() {
            if pat.regex.is_match(val) {
                return Some(SanitizationReason::SensitiveValuePattern(
                    pat.name.to_string(),
                ));
            }
        }

        None
    }

    /// Determines if a key-value pair is sensitive.
    pub fn is_sensitive(&self, key: &str, val: &str) -> Option<SanitizationReason> {
        let upper = key.to_uppercase();

        // Custom allowed override takes precedence unless explicitly blocked
        if self.custom_allowed_keys.contains(&upper) {
            if self.custom_blocked_keys.contains(&upper) {
                return Some(SanitizationReason::CustomBlocked);
            }
            return None;
        }

        // Allowlist policy check
        if self.policy == SanitizationPolicy::Allowlist {
            if !SAFE_SYSTEM_VARIABLES.contains(upper.as_str())
                && !self.custom_allowed_keys.contains(&upper)
            {
                return Some(SanitizationReason::NotInAllowlist);
            }
        }

        // Check key name
        if let Some(reason) = self.check_key_sensitivity(key) {
            return Some(reason);
        }

        // Check value
        if let Some(reason) = self.check_value_sensitivity(val) {
            return Some(reason);
        }

        None
    }

    /// Ensures essential system variables like PATH and Windows SYSTEMROOT
    /// are present so child processes do not fail to launch.
    fn ensure_essential_vars(clean_env: &mut HashMap<String, String>) {
        let has_path = clean_env.keys().any(|k| k.eq_ignore_ascii_case("PATH"));
        if !has_path {
            #[cfg(windows)]
            const DEFAULT_PATH: &str = "C:\\Windows\\system32;C:\\Windows";
            #[cfg(not(windows))]
            const DEFAULT_PATH: &str = "/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin";

            clean_env.insert("PATH".to_string(), DEFAULT_PATH.to_string());
        }

        #[cfg(windows)]
        {
            let has_sysroot = clean_env
                .keys()
                .any(|k| k.eq_ignore_ascii_case("SYSTEMROOT"));
            if !has_sysroot {
                clean_env.insert("SYSTEMROOT".to_string(), "C:\\Windows".to_string());
            }
            let has_comspec = clean_env.keys().any(|k| k.eq_ignore_ascii_case("COMSPEC"));
            if !has_comspec {
                clean_env.insert(
                    "COMSPEC".to_string(),
                    "C:\\Windows\\system32\\cmd.exe".to_string(),
                );
            }
        }
    }

    /// Sanitizes an environment map, stripping all sensitive variables.
    pub fn sanitize_map(&self, env: &HashMap<String, String>) -> SanitizationResult {
        let mut clean_env = HashMap::with_capacity(env.len());
        let mut stripped = Vec::new();

        for (k, v) in env {
            if let Some(reason) = self.is_sensitive(k, v) {
                stripped.push(SanitizationReport {
                    key: k.clone(),
                    reason,
                    masked_value: mask_value(v),
                });
            } else {
                clean_env.insert(k.clone(), v.clone());
            }
        }

        // Ensure essential execution variables exist
        Self::ensure_essential_vars(&mut clean_env);

        SanitizationResult {
            clean_env,
            stripped,
        }
    }

    /// Returns a clean HashMap of environment variables with secrets stripped.
    pub fn clean_env(&self, env: &HashMap<String, String>) -> HashMap<String, String> {
        self.sanitize_map(env).clean_env
    }

    /// Sanitizes the current process environment (`std::env::vars()`).
    pub fn sanitize_current_env(&self) -> HashMap<String, String> {
        let current: HashMap<String, String> = std::env::vars().collect();
        self.clean_env(&current)
    }

    /// Prepares and applies a clean, sanitized environment to a `tokio::process::Command`.
    pub fn apply_to_tokio_command(
        &self,
        cmd: &mut tokio::process::Command,
        extra_env: Option<&HashMap<String, String>>,
    ) -> SanitizationResult {
        let mut merged: HashMap<String, String> = std::env::vars().collect();
        if let Some(extra) = extra_env {
            for (k, v) in extra {
                merged.insert(k.clone(), v.clone());
            }
        }

        let result = self.sanitize_map(&merged);

        cmd.env_clear();
        cmd.envs(&result.clean_env);

        result
    }

    /// Applies an exact environment map to a `tokio::process::Command` without merging host `std::env::vars()`.
    pub fn apply_exact_to_tokio_command(
        &self,
        cmd: &mut tokio::process::Command,
        env: &HashMap<String, String>,
    ) -> SanitizationResult {
        let result = self.sanitize_map(env);

        cmd.env_clear();
        cmd.envs(&result.clean_env);

        result
    }

    /// Prepares and applies a clean, sanitized environment to a `std::process::Command`.
    pub fn apply_to_std_command(
        &self,
        cmd: &mut std::process::Command,
        extra_env: Option<&HashMap<String, String>>,
    ) -> SanitizationResult {
        let mut merged: HashMap<String, String> = std::env::vars().collect();
        if let Some(extra) = extra_env {
            for (k, v) in extra {
                merged.insert(k.clone(), v.clone());
            }
        }

        let result = self.sanitize_map(&merged);

        cmd.env_clear();
        cmd.envs(&result.clean_env);

        result
    }

    /// Applies an exact environment map to a `std::process::Command` without merging host `std::env::vars()`.
    pub fn apply_exact_to_std_command(
        &self,
        cmd: &mut std::process::Command,
        env: &HashMap<String, String>,
    ) -> SanitizationResult {
        let result = self.sanitize_map(env);

        cmd.env_clear();
        cmd.envs(&result.clean_env);

        result
    }

    /// Redacts sensitive values in arbitrary strings.
    pub fn redact_text(&self, text: &str) -> String {
        self.scrubber.redact_text(text)
    }

    /// Redacts sensitive values in tool outputs before presentation to models or users.
    pub fn redact_tool_output(&self, output: &str) -> String {
        self.scrubber.redact_text(output)
    }

    /// Redacts sensitive values in error logs.
    pub fn redact_error_log(&self, log: &str) -> String {
        self.scrubber.redact_text(log)
    }

    /// Redacts sensitive values within JSON structures.
    pub fn redact_json(&self, val: &Value) -> Value {
        self.scrubber.redact_json(val)
    }
}

// ===========================================================================
// Convenience Free Functions
// ===========================================================================

/// Safely masks a sensitive value for audit logging or display.
///
/// Values are never displayed in full:
/// - 6 or fewer chars: `"***"`
/// - 7 to 12 chars: `"ab***yz"`
/// - More than 12 chars: `"abcd...wxyz"`
pub fn mask_value(val: &str) -> String {
    let len = val.chars().count();
    if len <= 6 {
        "***".to_string()
    } else if len <= 12 {
        let chars: Vec<char> = val.chars().collect();
        let prefix: String = chars.iter().take(2).collect();
        let suffix: String = chars.iter().rev().take(2).rev().collect();
        format!("{prefix}***{suffix}")
    } else {
        let chars: Vec<char> = val.chars().collect();
        let prefix: String = chars.iter().take(4).collect();
        let suffix: String = chars.iter().rev().take(4).rev().collect();
        format!("{prefix}...{suffix}")
    }
}

/// Convenience standalone function to sanitize an environment map using default rules.
pub fn sanitize_env(env: &HashMap<String, String>) -> HashMap<String, String> {
    EnvCleaner::default().clean_env(env)
}

/// Convenience standalone function to check if a key name is considered sensitive.
pub fn is_sensitive_key(key: &str) -> bool {
    EnvCleaner::default().check_key_sensitivity(key).is_some()
}

/// Convenience standalone function to check if a value appears to contain credentials.
pub fn is_sensitive_value(val: &str) -> bool {
    EnvCleaner::default().check_value_sensitivity(val).is_some()
}

/// Convenience standalone function to check if a key-value pair is sensitive.
pub fn is_sensitive(key: &str, val: &str) -> bool {
    EnvCleaner::default().is_sensitive(key, val).is_some()
}

/// Convenience standalone function to redact secrets in arbitrary strings.
pub fn redact_secrets(text: &str) -> String {
    SecretScrubber::default().redact_text(text)
}

/// Convenience standalone function to redact secrets in strings and return findings.
pub fn redact_secrets_with_findings(text: &str) -> (String, Vec<SecretFinding>) {
    SecretScrubber::default().redact_text_with_findings(text)
}

/// Convenience standalone function to scan a string for sensitive secrets.
pub fn find_secrets(text: &str) -> Vec<SecretFinding> {
    SecretScrubber::default().find_secrets(text)
}

/// Convenience standalone function to redact secrets in tool outputs.
pub fn redact_tool_output(output: &str) -> String {
    SecretScrubber::default().redact_text(output)
}

/// Convenience standalone function to redact secrets in error logs.
pub fn redact_error_log(log: &str) -> String {
    SecretScrubber::default().redact_text(log)
}

/// Convenience standalone function to redact secrets in JSON values.
pub fn redact_json_secrets(val: &Value) -> Value {
    SecretScrubber::default().redact_json(val)
}

// ===========================================================================
// Tool Trait Implementation (EnvCleanerTool)
// ===========================================================================

/// Tool providing environment sanitization and secret scrubbing capabilities.
#[derive(Debug, Clone, Default)]
pub struct EnvCleanerTool {
    cleaner: EnvCleaner,
    scrubber: SecretScrubber,
}

/// Type alias for `EnvCleanerTool` matching alternative naming conventions.
pub type EnvSanitizerTool = EnvCleanerTool;

/// Type alias for `EnvCleanerTool` matching secret scrubber naming conventions.
pub type SecretScrubberTool = EnvCleanerTool;

impl EnvCleanerTool {
    /// Creates a new `EnvCleanerTool` with default settings.
    pub fn new() -> Self {
        Self {
            cleaner: EnvCleaner::new(),
            scrubber: SecretScrubber::new(),
        }
    }

    /// Creates an `EnvCleanerTool` with a customized `EnvCleaner`.
    pub fn with_cleaner(cleaner: EnvCleaner) -> Self {
        let scrubber = cleaner.scrubber().clone();
        Self { cleaner, scrubber }
    }

    /// Creates an `EnvCleanerTool` with a customized `SecretScrubber`.
    pub fn with_scrubber(scrubber: SecretScrubber) -> Self {
        let cleaner = EnvCleaner::new().with_scrubber(scrubber.clone());
        Self { cleaner, scrubber }
    }
}

#[async_trait]
impl Tool for EnvCleanerTool {
    fn name(&self) -> &str {
        "env_cleaner"
    }

    fn description(&self) -> &str {
        "Sanitize environment variables, scrub sensitive credentials, and redact API keys (OpenAI, Anthropic, Google, AWS, GitHub, Slack, Stripe), bearer tokens, private keys, database URLs, and passwords from strings, tool outputs, and logs."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "Operation to perform: 'redact_text' (scrub secrets from string/logs), 'sanitize_env' (filter secret env vars), 'scan_secrets' (inspect and locate secrets), 'check_key' (test if a variable name is sensitive), 'check_value' (test if a value contains credentials), 'redact_json' (recursively scrub secrets in JSON), or 'audit_env' (audit current environment).",
                    "enum": [
                        "redact_text",
                        "sanitize_env",
                        "scan_secrets",
                        "check_key",
                        "check_value",
                        "redact_json",
                        "audit_env"
                    ]
                },
                "text": {
                    "type": "string",
                    "description": "Text, tool output, or error log to scan or redact."
                },
                "env": {
                    "type": "object",
                    "description": "Environment variable map (key-value object) to sanitize."
                },
                "key": {
                    "type": "string",
                    "description": "Single environment variable key name to check for sensitivity."
                },
                "value": {
                    "type": "string",
                    "description": "Single value or secret to check for sensitivity."
                },
                "json_data": {
                    "description": "JSON string, object, or array to scrub of secrets.",
                    "oneOf": [
                        { "type": "string" },
                        { "type": "object" },
                        { "type": "array" }
                    ]
                },
                "policy": {
                    "type": "string",
                    "description": "Sanitization policy mode: 'blocklist' (default, strip secrets) or 'allowlist' (strict, only allow safe system vars).",
                    "enum": ["blocklist", "allowlist"]
                },
                "placeholder_style": {
                    "type": "string",
                    "description": "Redaction replacement style: 'type_specific' (e.g. [REDACTED_API_KEY]), 'simple' ([REDACTED]), 'generic', or 'masked' (e.g. sk-p...3456).",
                    "enum": ["type_specific", "simple", "generic", "masked"]
                },
                "custom_blocked_keys": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional list of custom environment variable names to block."
                },
                "custom_allowed_keys": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional list of custom environment variable names to explicitly allow."
                },
                "custom_secrets": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional list of specific sensitive strings to always redact."
                }
            }
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> anyhow::Result<String> {
        let mut cleaner = self.cleaner.clone();
        let mut scrubber = self.scrubber.clone();

        // Parse policy
        if let Some(policy_str) = args.get("policy").and_then(|v| v.as_str()) {
            match policy_str.to_lowercase().as_str() {
                "allowlist" | "strict" => {
                    cleaner = cleaner.with_policy(SanitizationPolicy::Allowlist);
                }
                "blocklist" => {
                    cleaner = cleaner.with_policy(SanitizationPolicy::Blocklist);
                }
                _ => {}
            }
        }

        // Parse placeholder style
        if let Some(style_str) = args.get("placeholder_style").and_then(|v| v.as_str()) {
            match style_str.to_lowercase().as_str() {
                "simple" => {
                    scrubber = scrubber.with_placeholder_style(PlaceholderStyle::Simple);
                }
                "generic" => {
                    scrubber = scrubber.with_placeholder_style(PlaceholderStyle::Generic);
                }
                "masked" => {
                    scrubber = scrubber.with_placeholder_style(PlaceholderStyle::Masked);
                }
                "type_specific" => {
                    scrubber = scrubber.with_placeholder_style(PlaceholderStyle::TypeSpecific);
                }
                _ => {}
            }
        }

        // Parse custom blocked keys
        if let Some(blocked) = args.get("custom_blocked_keys").and_then(|v| v.as_array()) {
            for item in blocked {
                if let Some(k) = item.as_str() {
                    cleaner = cleaner.with_blocked_key(k);
                }
            }
        }

        // Parse custom allowed keys
        if let Some(allowed) = args.get("custom_allowed_keys").and_then(|v| v.as_array()) {
            for item in allowed {
                if let Some(k) = item.as_str() {
                    cleaner = cleaner.with_allowed_key(k);
                }
            }
        }

        // Parse custom secrets
        if let Some(secrets) = args.get("custom_secrets").and_then(|v| v.as_array()) {
            for item in secrets {
                if let Some(s) = item.as_str() {
                    scrubber = scrubber.with_known_secret(s);
                }
            }
        }

        // Determine action
        let action = if let Some(a) = args.get("action").and_then(|v| v.as_str()) {
            a.to_string()
        } else if args.get("text").is_some() {
            "redact_text".to_string()
        } else if args.get("env").is_some() {
            "sanitize_env".to_string()
        } else if args.get("key").is_some() && args.get("value").is_none() {
            "check_key".to_string()
        } else if args.get("value").is_some() && args.get("key").is_none() {
            "check_value".to_string()
        } else if args.get("json_data").is_some() {
            "redact_json".to_string()
        } else {
            "audit_env".to_string()
        };

        match action.as_str() {
            "redact_text" => {
                let input_text = args
                    .get("text")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();

                let (redacted, findings) = scrubber.redact_text_with_findings(input_text);
                let findings_count = findings.len();
                let is_clean = findings.is_empty();

                let response = json!({
                    "action": "redact_text",
                    "is_clean": is_clean,
                    "findings_count": findings_count,
                    "findings": findings,
                    "redacted_text": redacted,
                });

                Ok(serde_json::to_string_pretty(&response)?)
            }

            "scan_secrets" => {
                let input_text = args
                    .get("text")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();

                let findings = scrubber.find_secrets(input_text);
                let findings_count = findings.len();
                let is_clean = findings.is_empty();

                let response = json!({
                    "action": "scan_secrets",
                    "is_clean": is_clean,
                    "findings_count": findings_count,
                    "findings": findings,
                });

                Ok(serde_json::to_string_pretty(&response)?)
            }

            "sanitize_env" => {
                let mut map = HashMap::new();
                if let Some(env_obj) = args.get("env").and_then(|v| v.as_object()) {
                    for (k, v) in env_obj {
                        if let Some(s) = v.as_str() {
                            map.insert(k.clone(), s.to_string());
                        } else {
                            map.insert(k.clone(), v.to_string());
                        }
                    }
                } else {
                    map = ctx.env.clone();
                }

                let result = cleaner.sanitize_map(&map);

                let response = json!({
                    "action": "sanitize_env",
                    "is_clean": result.is_clean(),
                    "total_input_vars": map.len(),
                    "clean_vars_count": result.clean_env.len(),
                    "stripped_count": result.stripped_count(),
                    "stripped": result.stripped,
                    "clean_env": result.clean_env,
                });

                Ok(serde_json::to_string_pretty(&response)?)
            }

            "audit_env" => {
                let result = cleaner.sanitize_map(&ctx.env);

                let response = json!({
                    "action": "audit_env",
                    "is_clean": result.is_clean(),
                    "total_vars": ctx.env.len(),
                    "clean_vars_count": result.clean_env.len(),
                    "stripped_count": result.stripped_count(),
                    "stripped": result.stripped,
                });

                Ok(serde_json::to_string_pretty(&response)?)
            }

            "check_key" => {
                let key = args.get("key").and_then(|v| v.as_str()).unwrap_or_default();

                let reason = cleaner.check_key_sensitivity(key);
                let is_sensitive = reason.is_some();

                let response = json!({
                    "action": "check_key",
                    "key": key,
                    "is_sensitive": is_sensitive,
                    "reason": reason,
                });

                Ok(serde_json::to_string_pretty(&response)?)
            }

            "check_value" => {
                let val = args
                    .get("value")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();

                let reason = cleaner.check_value_sensitivity(val);
                let is_sensitive = reason.is_some();

                let response = json!({
                    "action": "check_value",
                    "is_sensitive": is_sensitive,
                    "reason": reason,
                    "masked_value": mask_value(val),
                });

                Ok(serde_json::to_string_pretty(&response)?)
            }

            "redact_json" => {
                let json_val = if let Some(v) = args.get("json_data") {
                    if let Some(s) = v.as_str() {
                        match serde_json::from_str::<Value>(s) {
                            Ok(parsed) => parsed,
                            Err(_) => Value::String(s.to_string()),
                        }
                    } else {
                        v.clone()
                    }
                } else {
                    Value::Null
                };

                let redacted = scrubber.redact_json(&json_val);

                let response = json!({
                    "action": "redact_json",
                    "redacted_json": redacted,
                });

                Ok(serde_json::to_string_pretty(&response)?)
            }

            other => {
                anyhow::bail!("Unknown action '{other}'. Supported actions: redact_text, sanitize_env, scan_secrets, check_key, check_value, redact_json, audit_env");
            }
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
    fn test_known_secret_keys_blocked() {
        let cleaner = EnvCleaner::new();

        let mut env = HashMap::new();
        env.insert(
            "OPENAI_API_KEY".to_string(),
            "sk-12345678901234567890".to_string(),
        );
        env.insert(
            "ANTHROPIC_API_KEY".to_string(),
            "sk-ant-12345678901234567890".to_string(),
        );
        env.insert(
            "GEMINI_API_KEY".to_string(),
            "AIzaSyD-12345678901234567890".to_string(),
        );
        env.insert(
            "AWS_SECRET_ACCESS_KEY".to_string(),
            "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_string(),
        );
        env.insert(
            "AWS_ACCESS_KEY_ID".to_string(),
            "AKIAIOSFODNN7EXAMPLE".to_string(),
        );
        env.insert(
            "GITHUB_TOKEN".to_string(),
            "ghp_123456789012345678901234567890123456".to_string(),
        );
        env.insert(
            "DATABASE_URL".to_string(),
            "postgres://user:pass@localhost:5432/db".to_string(),
        );
        env.insert("PATH".to_string(), "/usr/bin:/bin".to_string());
        env.insert("USER".to_string(), "developer".to_string());
        env.insert("HOME".to_string(), "/home/developer".to_string());

        let result = cleaner.sanitize_map(&env);

        assert_eq!(result.stripped_count(), 7);
        assert!(result.has_stripped("OPENAI_API_KEY"));
        assert!(result.has_stripped("ANTHROPIC_API_KEY"));
        assert!(result.has_stripped("GEMINI_API_KEY"));
        assert!(result.has_stripped("AWS_SECRET_ACCESS_KEY"));
        assert!(result.has_stripped("AWS_ACCESS_KEY_ID"));
        assert!(result.has_stripped("GITHUB_TOKEN"));
        assert!(result.has_stripped("DATABASE_URL"));

        assert_eq!(
            result.clean_env.get("PATH"),
            Some(&"/usr/bin:/bin".to_string())
        );
        assert_eq!(result.clean_env.get("USER"), Some(&"developer".to_string()));
        assert_eq!(
            result.clean_env.get("HOME"),
            Some(&"/home/developer".to_string())
        );
    }

    #[test]
    fn test_case_insensitive_matching() {
        let cleaner = EnvCleaner::new();

        let mut env = HashMap::new();
        env.insert(
            "openai_api_key".to_string(),
            "sk-12345678901234567890".to_string(),
        );
        env.insert("Anthropic_Api_Key".to_string(), "secret".to_string());
        env.insert("GITHUB_token".to_string(), "token".to_string());

        let result = cleaner.sanitize_map(&env);
        assert_eq!(result.stripped_count(), 3);
        assert!(!result.clean_env.contains_key("openai_api_key"));
        assert!(!result.clean_env.contains_key("Anthropic_Api_Key"));
        assert!(!result.clean_env.contains_key("GITHUB_token"));
    }

    #[test]
    fn test_sensitive_suffixes_and_prefixes() {
        let cleaner = EnvCleaner::new();

        let mut env = HashMap::new();
        env.insert("CUSTOM_SERVICE_API_KEY".to_string(), "secret1".to_string());
        env.insert("MY_AUTH_TOKEN".to_string(), "secret2".to_string());
        env.insert("REDIS_PASSWORD".to_string(), "secret3".to_string());
        env.insert("SECRET_KEY_CONFIG".to_string(), "secret4".to_string());
        env.insert("PRIVATE_CERTIFICATE".to_string(), "secret5".to_string());
        env.insert("SAFE_CONFIG_NAME".to_string(), "app-v1".to_string());

        let result = cleaner.sanitize_map(&env);
        assert_eq!(result.stripped_count(), 5);
        assert_eq!(
            result.clean_env.get("SAFE_CONFIG_NAME"),
            Some(&"app-v1".to_string())
        );
    }

    #[test]
    fn test_safe_system_variables_preserved() {
        let cleaner = EnvCleaner::new();

        let mut env = HashMap::new();
        env.insert("PWD".to_string(), "/workspace/project".to_string());
        env.insert("OLDPWD".to_string(), "/workspace".to_string());
        env.insert("TERM".to_string(), "xterm-256color".to_string());
        env.insert("LANG".to_string(), "en_US.UTF-8".to_string());
        env.insert("SHELL".to_string(), "/bin/bash".to_string());
        env.insert(
            "SSL_CERT_FILE".to_string(),
            "/etc/ssl/certs/ca-certificates.crt".to_string(),
        );
        env.insert("PATH".to_string(), "/usr/bin:/bin".to_string());

        let result = cleaner.sanitize_map(&env);
        assert_eq!(result.stripped_count(), 0);
        assert_eq!(
            result.clean_env.get("PWD"),
            Some(&"/workspace/project".to_string())
        );
        assert_eq!(
            result.clean_env.get("OLDPWD"),
            Some(&"/workspace".to_string())
        );
        assert_eq!(
            result.clean_env.get("TERM"),
            Some(&"xterm-256color".to_string())
        );
        assert_eq!(
            result.clean_env.get("LANG"),
            Some(&"en_US.UTF-8".to_string())
        );
        assert_eq!(
            result.clean_env.get("SHELL"),
            Some(&"/bin/bash".to_string())
        );
        assert_eq!(
            result.clean_env.get("SSL_CERT_FILE"),
            Some(&"/etc/ssl/certs/ca-certificates.crt".to_string())
        );
        assert_eq!(
            result.clean_env.get("PATH"),
            Some(&"/usr/bin:/bin".to_string())
        );
    }

    #[test]
    fn test_value_scanning_heuristics() {
        let cleaner = EnvCleaner::new();

        let mut env = HashMap::new();
        env.insert(
            "INLINE_CONFIG".to_string(),
            "sk-proj-abc12345678901234567890".to_string(),
        );
        env.insert(
            "MY_VAR".to_string(),
            "ghp_123456789012345678901234567890123456".to_string(),
        );
        env.insert("AWS_VAR".to_string(), "AKIAIOSFODNN7EXAMPLE".to_string());
        env.insert(
            "PEM_CERT".to_string(),
            "-----BEGIN RSA PRIVATE KEY-----\nMIIE...".to_string(),
        );
        env.insert("INNOCENT_VAL".to_string(), "regular-text-value".to_string());

        let result = cleaner.sanitize_map(&env);
        assert_eq!(result.stripped_count(), 4);
        assert!(result.has_stripped("INLINE_CONFIG"));
        assert!(result.has_stripped("MY_VAR"));
        assert!(result.has_stripped("AWS_VAR"));
        assert!(result.has_stripped("PEM_CERT"));
        assert_eq!(
            result.clean_env.get("INNOCENT_VAL"),
            Some(&"regular-text-value".to_string())
        );
    }

    #[test]
    fn test_masking() {
        assert_eq!(mask_value("short"), "***");
        assert_eq!(mask_value("secret12"), "se***12");
        assert_eq!(mask_value("sk-proj-1234567890123456"), "sk-p...3456");
    }

    #[test]
    fn test_custom_blocked_and_allowed() {
        let cleaner = EnvCleaner::new()
            .with_blocked_key("CUSTOM_SECRET")
            .with_allowed_key("OPENAI_API_KEY");

        let mut env = HashMap::new();
        env.insert("CUSTOM_SECRET".to_string(), "123".to_string());
        env.insert("OPENAI_API_KEY".to_string(), "override-value".to_string());

        let result = cleaner.sanitize_map(&env);
        assert_eq!(result.stripped_count(), 1);
        assert!(result.has_stripped("CUSTOM_SECRET"));
        assert_eq!(
            result.clean_env.get("OPENAI_API_KEY"),
            Some(&"override-value".to_string())
        );
    }

    #[test]
    fn test_strict_allowlist_policy() {
        let cleaner = EnvCleaner::strict().with_allowed_key("CUSTOM_ALLOWED");

        let mut env = HashMap::new();
        env.insert("PATH".to_string(), "/usr/bin".to_string());
        env.insert("CUSTOM_ALLOWED".to_string(), "custom".to_string());
        env.insert("RANDOM_VARIABLE".to_string(), "random".to_string());

        let result = cleaner.sanitize_map(&env);
        assert_eq!(result.stripped_count(), 1);
        assert!(result.has_stripped("RANDOM_VARIABLE"));
        assert_eq!(result.clean_env.get("PATH"), Some(&"/usr/bin".to_string()));
        assert_eq!(
            result.clean_env.get("CUSTOM_ALLOWED"),
            Some(&"custom".to_string())
        );
    }

    #[tokio::test]
    async fn test_apply_exact_to_tokio_command() {
        let cleaner = EnvCleaner::new();
        let mut cmd = tokio::process::Command::new("echo");

        let mut env = HashMap::new();
        env.insert(
            "OPENAI_API_KEY".to_string(),
            "sk-secret123456789012345".to_string(),
        );
        env.insert("SAFE_VAR".to_string(), "safe-value".to_string());

        let result = cleaner.apply_exact_to_tokio_command(&mut cmd, &env);
        assert_eq!(result.stripped_count(), 1);
        assert!(result.clean_env.contains_key("SAFE_VAR"));
        assert!(!result.clean_env.contains_key("OPENAI_API_KEY"));
    }

    #[test]
    fn test_all_ai_providers_blocked() {
        let cleaner = EnvCleaner::new();
        let keys = [
            "OPENAI_API_KEY",
            "OPENAI_ORG_ID",
            "OPENAI_PROJECT_ID",
            "ANTHROPIC_API_KEY",
            "CLAUDE_API_KEY",
            "GEMINI_API_KEY",
            "GOOGLE_API_KEY",
            "GOOGLE_APPLICATION_CREDENTIALS",
            "MISTRAL_API_KEY",
            "GROQ_API_KEY",
            "COHERE_API_KEY",
            "COHERE_TOKEN",
            "TOGETHER_API_KEY",
            "FIREWORKS_API_KEY",
            "DEEPSEEK_API_KEY",
            "PERPLEXITY_API_KEY",
            "PPLX_API_KEY",
            "VOYAGE_API_KEY",
            "OPENROUTER_API_KEY",
            "AI21_API_KEY",
            "REPLICATE_API_TOKEN",
            "HF_TOKEN",
            "HUGGINGFACE_TOKEN",
            "OLLAMA_API_KEY",
            "XAI_API_KEY",
        ];

        for &key in &keys {
            assert!(
                cleaner.check_key_sensitivity(key).is_some(),
                "Expected AI provider key '{}' to be flagged as sensitive",
                key
            );
        }
    }

    #[test]
    fn test_cloud_and_vcs_keys_blocked() {
        let cleaner = EnvCleaner::new();
        let keys = [
            "AWS_ACCESS_KEY_ID",
            "AWS_SECRET_ACCESS_KEY",
            "AWS_SESSION_TOKEN",
            "AZURE_OPENAI_API_KEY",
            "AZURE_API_KEY",
            "AZURE_CLIENT_SECRET",
            "DIGITALOCEAN_ACCESS_TOKEN",
            "CLOUDFLARE_API_TOKEN",
            "CF_API_TOKEN",
            "VERCEL_TOKEN",
            "NETLIFY_AUTH_TOKEN",
            "HEROKU_API_KEY",
            "FLY_API_TOKEN",
            "GITHUB_TOKEN",
            "GH_TOKEN",
            "GITHUB_PAT",
            "GITLAB_TOKEN",
            "BITBUCKET_TOKEN",
            "GIT_PASSWORD",
            "GIT_ASKPASS",
        ];

        for &key in &keys {
            assert!(
                cleaner.check_key_sensitivity(key).is_some(),
                "Expected cloud/vcs key '{}' to be flagged as sensitive",
                key
            );
        }
    }

    #[test]
    fn test_databases_and_secrets_managers_blocked() {
        let cleaner = EnvCleaner::new();
        let keys = [
            "DATABASE_URL",
            "DATABASE_PASSWORD",
            "DB_PASSWORD",
            "POSTGRES_PASSWORD",
            "PGPASSWORD",
            "MYSQL_PWD",
            "MYSQL_ROOT_PASSWORD",
            "REDIS_PASSWORD",
            "REDIS_AUTH",
            "MONGO_URI",
            "MONGODB_URI",
            "VAULT_TOKEN",
            "BW_SESSION",
            "OP_SERVICE_ACCOUNT_TOKEN",
            "ONEPASSWORD_TOKEN",
            "STRIPE_SECRET_KEY",
            "STRIPE_API_KEY",
            "SENDGRID_API_KEY",
            "TWILIO_AUTH_TOKEN",
        ];

        for &key in &keys {
            assert!(
                cleaner.check_key_sensitivity(key).is_some(),
                "Expected database/secret key '{}' to be flagged as sensitive",
                key
            );
        }
    }

    #[test]
    fn test_jwt_and_bearer_values_blocked() {
        let cleaner = EnvCleaner::new();
        let mut env = HashMap::new();
        env.insert(
            "AUTH_HEADER".to_string(),
            "Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.doNotLeakThisSignature".to_string(),
        );
        env.insert(
            "SESSION_JWT".to_string(),
            "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIn0.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c".to_string(),
        );
        let slack_tok = format!(
            "xoxb-{}-{}",
            "123456789012-1234567890123", "abcdefghijklmnopqrstuvwx"
        );
        let stripe_tok = format!("sk_live_{}", "51Abcdefghijklmnopqrstuvwx");
        env.insert("SLACK_KEY".to_string(), slack_tok);
        env.insert("STRIPE_KEY".to_string(), stripe_tok);

        let result = cleaner.sanitize_map(&env);
        assert_eq!(result.stripped_count(), 4);
        assert!(result.has_stripped("AUTH_HEADER"));
        assert!(result.has_stripped("SESSION_JWT"));
        assert!(result.has_stripped("SLACK_KEY"));
        assert!(result.has_stripped("STRIPE_KEY"));
    }

    #[test]
    fn test_custom_blocked_pattern_regex() {
        let cleaner =
            EnvCleaner::new().with_blocked_pattern(Regex::new(r"(?i)^INTERNAL_SECRET_.*").unwrap());
        let mut env = HashMap::new();
        env.insert("INTERNAL_SECRET_FOO".to_string(), "val1".to_string());
        env.insert("internal_secret_bar".to_string(), "val2".to_string());
        env.insert("INTERNAL_PUBLIC_BAZ".to_string(), "val3".to_string());

        let result = cleaner.sanitize_map(&env);
        assert_eq!(result.stripped_count(), 2);
        assert!(result.has_stripped("INTERNAL_SECRET_FOO"));
        assert!(result.has_stripped("internal_secret_bar"));
        assert_eq!(
            result.clean_env.get("INTERNAL_PUBLIC_BAZ"),
            Some(&"val3".to_string())
        );
    }

    #[test]
    fn test_essential_path_fallback() {
        let cleaner = EnvCleaner::new();
        let empty_env = HashMap::new();
        let result = cleaner.sanitize_map(&empty_env);

        assert!(result.clean_env.contains_key("PATH") || result.clean_env.contains_key("Path"));
    }

    #[test]
    fn test_sanitization_result_methods() {
        let mut env = HashMap::new();
        env.insert("SAFE".to_string(), "hello".to_string());

        let res1 = EnvCleaner::new().sanitize_map(&env);
        assert!(res1.is_clean());
        assert_eq!(res1.stripped_count(), 0);
        assert!(!res1.has_stripped("SAFE"));

        env.insert(
            "OPENAI_API_KEY".to_string(),
            "sk-12345678901234567890".to_string(),
        );
        let res2 = EnvCleaner::new().sanitize_map(&env);
        assert!(!res2.is_clean());
        assert_eq!(res2.stripped_count(), 1);
        assert!(res2.has_stripped("openai_api_key"));
    }

    // =======================================================================
    // Secret Scrubber & Redactor Unit Tests
    // =======================================================================

    #[test]
    fn test_redact_openai_and_anthropic_api_keys() {
        let scrubber = SecretScrubber::new();

        let text =
            "OpenAI key: sk-proj-12345678901234567890, Anthropic key: sk-ant-12345678901234567890.";
        let redacted = scrubber.redact_text(text);

        assert_eq!(
            redacted,
            "OpenAI key: [REDACTED_OPENAI_API_KEY], Anthropic key: [REDACTED_ANTHROPIC_API_KEY]."
        );
    }

    #[test]
    fn test_redact_google_github_and_aws_keys() {
        let scrubber = SecretScrubber::new();

        let text = "Google: AIzaSyD-12345678901234567890abcdefghij, GitHub: ghp_123456789012345678901234567890123456, AWS: AKIAIOSFODNN7EXAMPLE";
        let redacted = scrubber.redact_text(text);

        assert_eq!(
            redacted,
            "Google: [REDACTED_GOOGLE_API_KEY], GitHub: [REDACTED_GITHUB_TOKEN], AWS: [REDACTED_AWS_ACCESS_KEY]"
        );
    }

    #[test]
    fn test_redact_bearer_tokens_and_jwts() {
        let scrubber = SecretScrubber::new();

        let text = "Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.doNotLeakSignature\nDirect JWT: eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.anotherSignature";
        let redacted = scrubber.redact_text(text);

        assert!(redacted.contains("Bearer [REDACTED_BEARER_TOKEN]"));
        assert!(redacted.contains("Direct JWT: [REDACTED_JWT_TOKEN]"));
        assert!(!redacted.contains("doNotLeakSignature"));
        assert!(!redacted.contains("anotherSignature"));
    }

    #[test]
    fn test_redact_database_urls() {
        let scrubber = SecretScrubber::new();

        let text = "Connecting to postgres://admin_user:super_secret_db_pass_123@db.prod.internal:5432/main_db ...";
        let redacted = scrubber.redact_text(text);

        assert_eq!(
            redacted,
            "Connecting to postgres://admin_user:[REDACTED_PASSWORD]@db.prod.internal:5432/main_db ..."
        );

        let redis_text = "Redis: redis://:redisSuperSecret@cache.internal:6379/0";
        let redis_redacted = scrubber.redact_text(redis_text);
        assert_eq!(
            redis_redacted,
            "Redis: redis://:[REDACTED_PASSWORD]@cache.internal:6379/0"
        );
    }

    #[test]
    fn test_redact_private_key_blocks() {
        let scrubber = SecretScrubber::new();

        let text = "Header\n-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEA0+abcde12345\n-----END RSA PRIVATE KEY-----\nFooter";
        let redacted = scrubber.redact_text(text);

        assert_eq!(redacted, "Header\n[REDACTED_PRIVATE_KEY]\nFooter");
    }

    #[test]
    fn test_redact_password_assignments() {
        let scrubber = SecretScrubber::new();

        let text = "config.password = 'superSecretPass';\ndb_pass = \"pass1234\";\nclient_secret = 9876543210;";
        let redacted = scrubber.redact_text(text);

        assert!(redacted.contains("config.password = '[REDACTED_PASSWORD]';"));
        assert!(redacted.contains("db_pass = \"[REDACTED_PASSWORD]\";"));
        assert!(redacted.contains("client_secret = [REDACTED_PASSWORD];"));
        assert!(!redacted.contains("superSecretPass"));
    }

    #[test]
    fn test_redact_query_parameters() {
        let scrubber = SecretScrubber::new();

        let text = "Fetch url https://api.example.com/data?api_key=mySecretKey123456789&page=1";
        let redacted = scrubber.redact_text(text);

        assert_eq!(
            redacted,
            "Fetch url https://api.example.com/data?api_key=[REDACTED_API_KEY]&page=1"
        );
    }

    #[test]
    fn test_redact_custom_known_secrets() {
        let scrubber = SecretScrubber::new().with_known_secret("InternalConfidential123");

        let text = "Leak prevention test: value is InternalConfidential123 in log line.";
        let redacted = scrubber.redact_text(text);

        assert_eq!(
            redacted,
            "Leak prevention test: value is [REDACTED_SECRET] in log line."
        );
    }

    #[test]
    fn test_redact_placeholder_styles() {
        let text = "Key is sk-proj-12345678901234567890";

        let simple_scrubber =
            SecretScrubber::new().with_placeholder_style(PlaceholderStyle::Simple);
        assert_eq!(simple_scrubber.redact_text(text), "Key is [REDACTED]");

        let generic_scrubber =
            SecretScrubber::new().with_placeholder_style(PlaceholderStyle::Generic);
        assert_eq!(
            generic_scrubber.redact_text(text),
            "Key is [REDACTED_API_KEY]"
        );

        let masked_scrubber =
            SecretScrubber::new().with_placeholder_style(PlaceholderStyle::Masked);
        let masked = masked_scrubber.redact_text(text);
        assert!(masked.starts_with("Key is sk-p..."));

        let custom_scrubber = SecretScrubber::new()
            .with_placeholder_style(PlaceholderStyle::Custom("<SECRET>".to_string()));
        assert_eq!(custom_scrubber.redact_text(text), "Key is <SECRET>");
    }

    #[test]
    fn test_find_secrets_reporting() {
        let scrubber = SecretScrubber::new();
        let text = "Line 1: safe\nLine 2: sk-proj-12345678901234567890\nLine 3: ghp_123456789012345678901234567890123456";

        let findings = scrubber.find_secrets(text);
        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].kind, SecretKind::OpenAiApiKey);
        assert_eq!(findings[0].line, 2);
        assert_eq!(findings[1].kind, SecretKind::GitHubToken);
        assert_eq!(findings[1].line, 3);
    }

    #[test]
    fn test_redact_json_structure() {
        let scrubber = SecretScrubber::new();

        let json_data = json!({
            "name": "myapp",
            "openai_api_key": "sk-proj-12345678901234567890",
            "server": {
                "db_url": "postgres://user:secretpass@localhost:5432/db",
                "port": 8080
            },
            "tokens": [
                "ghp_123456789012345678901234567890123456",
                "safe_item"
            ]
        });

        let redacted = scrubber.redact_json(&json_data);

        assert_eq!(redacted["name"], "myapp");
        assert_eq!(redacted["openai_api_key"], "[REDACTED_API_KEY]");
        assert_eq!(
            redacted["server"]["db_url"],
            "postgres://user:[REDACTED_PASSWORD]@localhost:5432/db"
        );
        assert_eq!(redacted["server"]["port"], 8080);
        assert_eq!(redacted["tokens"][0], "[REDACTED_GITHUB_TOKEN]");
        assert_eq!(redacted["tokens"][1], "safe_item");
    }

    #[test]
    fn test_additional_providers_redaction() {
        let scrubber = SecretScrubber::new();

        let twilio_key = format!("SK{}", "1234567890abcdef1234567890abcdef");
        let text = format!("GitLab: glpat-12345678901234567890, HuggingFace: hf_1234567890123456789012345678901234, NPM: npm_1234567890123456789012345678901234, PyPI: pypi-AgEIcHlwaS5vcmcxMjM0NTY3ODkwMTIzNDU2Nzg5MDEyMzQ1Njc4OTA, SendGrid: SG.1234567890123456789012.1234567890123456789012345678901234567890123, Twilio: {}", twilio_key);
        let redacted = scrubber.redact_text(&text);
        assert!(redacted.contains("[REDACTED_GITLAB_TOKEN]"));
        assert!(redacted.contains("[REDACTED_HF_TOKEN]"));
        assert!(redacted.contains("[REDACTED_NPM_TOKEN]"));
        assert!(redacted.contains("[REDACTED_PYPI_TOKEN]"));
        assert!(redacted.contains("[REDACTED_SENDGRID_KEY]"));
        assert!(redacted.contains("[REDACTED_TWILIO_KEY]"));
    }

    // =======================================================================
    // Tool Trait Execution Unit Tests
    // =======================================================================

    #[tokio::test]
    async fn test_tool_redact_text_execution() {
        let tool = EnvCleanerTool::new();
        let ctx = ToolContext::default();

        let args = json!({
            "action": "redact_text",
            "text": "Log error: failed to authenticate with OpenAI API key sk-proj-12345678901234567890."
        });

        let output_str = tool
            .execute(args, &ctx)
            .await
            .expect("tool execution failed");
        let parsed: Value = serde_json::from_str(&output_str).expect("valid JSON response");

        assert_eq!(parsed["action"], "redact_text");
        assert_eq!(parsed["is_clean"], false);
        assert_eq!(parsed["findings_count"], 1);
        assert!(parsed["redacted_text"]
            .as_str()
            .unwrap()
            .contains("[REDACTED_OPENAI_API_KEY]"));
    }

    #[tokio::test]
    async fn test_tool_scan_secrets_execution() {
        let tool = EnvCleanerTool::new();
        let ctx = ToolContext::default();

        let args = json!({
            "action": "scan_secrets",
            "text": "Credentials:\nAWS: AKIAIOSFODNN7EXAMPLE\nGitHub: ghp_123456789012345678901234567890123456"
        });

        let output_str = tool
            .execute(args, &ctx)
            .await
            .expect("tool execution failed");
        let parsed: Value = serde_json::from_str(&output_str).expect("valid JSON response");

        assert_eq!(parsed["action"], "scan_secrets");
        assert_eq!(parsed["findings_count"], 2);
        assert_eq!(parsed["is_clean"], false);
    }

    #[tokio::test]
    async fn test_tool_sanitize_env_execution() {
        let tool = EnvCleanerTool::new();
        let ctx = ToolContext::default();

        let args = json!({
            "action": "sanitize_env",
            "env": {
                "OPENAI_API_KEY": "sk-12345678901234567890",
                "APP_ENV": "production",
                "PATH": "/bin"
            }
        });

        let output_str = tool
            .execute(args, &ctx)
            .await
            .expect("tool execution failed");
        let parsed: Value = serde_json::from_str(&output_str).expect("valid JSON response");

        assert_eq!(parsed["action"], "sanitize_env");
        assert_eq!(parsed["stripped_count"], 1);
        assert_eq!(parsed["clean_env"]["APP_ENV"], "production");
        assert!(parsed["clean_env"].get("OPENAI_API_KEY").is_none());
    }

    #[tokio::test]
    async fn test_tool_check_key_and_value() {
        let tool = EnvCleanerTool::new();
        let ctx = ToolContext::default();

        let check_key_args = json!({
            "action": "check_key",
            "key": "ANTHROPIC_API_KEY"
        });
        let out_key = tool
            .execute(check_key_args, &ctx)
            .await
            .expect("check_key failed");
        let parsed_key: Value = serde_json::from_str(&out_key).unwrap();
        assert_eq!(parsed_key["is_sensitive"], true);

        let check_val_args = json!({
            "action": "check_value",
            "value": "sk-ant-12345678901234567890"
        });
        let out_val = tool
            .execute(check_val_args, &ctx)
            .await
            .expect("check_value failed");
        let parsed_val: Value = serde_json::from_str(&out_val).unwrap();
        assert_eq!(parsed_val["is_sensitive"], true);
    }

    #[tokio::test]
    async fn test_tool_redact_json_execution() {
        let tool = EnvCleanerTool::new();
        let ctx = ToolContext::default();

        let args = json!({
            "action": "redact_json",
            "json_data": {
                "secret_key": "supersecretvalue123",
                "public_name": "fusion"
            }
        });

        let output_str = tool.execute(args, &ctx).await.expect("redact_json failed");
        let parsed: Value = serde_json::from_str(&output_str).expect("valid JSON response");

        assert_eq!(parsed["action"], "redact_json");
        assert_eq!(parsed["redacted_json"]["secret_key"], "[REDACTED_API_KEY]");
        assert_eq!(parsed["redacted_json"]["public_name"], "fusion");
    }
}
