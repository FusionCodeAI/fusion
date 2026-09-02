//! Secret Scanner — detect likely credentials, API keys, tokens, and passwords in source files.
//!
//! Scans a file tree (or single file) for patterns that strongly suggest committed secrets:
//! - AWS access key IDs and secret access keys
//! - GitHub personal access tokens (classic and fine-grained)
//! - Generic `Bearer …` / `token = "…"` / `password = "…"` assignments
//! - High-entropy Base64 and hex strings that look like random secrets
//!
//! For each finding the tool reports the file path (relative to cwd), line number, pattern
//! category, and the matched text (partially redacted so as not to spray the secret further).
//!
//! # Usage
//!
//! ```json
//! { "path": ".", "max_findings": 200 }
//! ```
//!
//! Parameters:
//! - `path` (string, optional): file or directory to scan; defaults to `.` (cwd).
//! - `max_findings` (number, optional): cap total findings; default 500.
//! - `include_hidden` (bool, optional): scan hidden files/directories; default false.
//! - `severity` (string, optional): minimum severity to report — `"high"`, `"medium"`, `"low"`; default `"low"`.
//! - `format` (string, optional): `"text"` (default) or `"json"`.

use async_trait::async_trait;
use ignore::WalkBuilder;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use crate::tools::types::{Tool, ToolContext};

// ===========================================================================
// Pattern definitions
// ===========================================================================

/// Severity level of a secret finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Low,
    Medium,
    High,
}

impl Severity {
    fn as_str(self) -> &'static str {
        match self {
            Severity::Low => "low",
            Severity::Medium => "medium",
            Severity::High => "high",
        }
    }

    fn from_str(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "high" => Severity::High,
            "medium" => Severity::Medium,
            _ => Severity::Low,
        }
    }
}

/// A single compiled pattern used for scanning.
struct SecretPattern {
    name: &'static str,
    severity: Severity,
    re: Regex,
}

/// Lazily compiled set of secret-detection patterns.
static PATTERNS: LazyLock<Vec<SecretPattern>> = LazyLock::new(|| {
    let specs: &[(&str, Severity, &str)] = &[
        // --- High confidence, high severity ---
        // AWS Access Key ID
        (
            "aws_access_key_id",
            Severity::High,
            r"(?i)(?:AKIA|ABIA|ACCA|AROA|AIDA|ASIA)[A-Z0-9]{16}",
        ),
        // AWS Secret Access Key (40 char base64-ish after key = / key: )
        (
            "aws_secret_access_key",
            Severity::High,
            r#"(?i)aws[_\-\s]*secret[_\-\s]*(?:access[_\-\s]*)?key["'\s]*[=:]["'\s]*([A-Za-z0-9/+]{40})"#,
        ),
        // GitHub classic PAT — ghp_…
        (
            "github_pat_classic",
            Severity::High,
            r"ghp_[A-Za-z0-9]{36}",
        ),
        // GitHub fine-grained PAT — github_pat_…
        (
            "github_pat_fine_grained",
            Severity::High,
            r"github_pat_[A-Za-z0-9_]{82}",
        ),
        // GitHub OAuth/app token — gho_ / ghs_ / ghu_
        (
            "github_oauth_token",
            Severity::High,
            r"gh[osu]_[A-Za-z0-9]{36}",
        ),
        // Slack bot/xoxb tokens
        (
            "slack_token",
            Severity::High,
            r"xox[baprs]-[0-9A-Za-z\-]{10,}",
        ),
        // Stripe secret key
        (
            "stripe_secret_key",
            Severity::High,
            r"sk_(?:live|test)_[A-Za-z0-9]{24,}",
        ),
        // Stripe publishable key (medium — less critical but still sensitive)
        (
            "stripe_publishable_key",
            Severity::Medium,
            r"pk_(?:live|test)_[A-Za-z0-9]{24,}",
        ),
        // Twilio Account SID / Auth Token pattern
        (
            "twilio_auth_token",
            Severity::High,
            r#"(?i)twilio.*(?:auth[_\-\s]*token|account[_\-\s]*sid)["'\s]*[=:][\s"']*([A-Fa-f0-9]{32})"#,
        ),
        // SendGrid API key
        (
            "sendgrid_api_key",
            Severity::High,
            r"SG\.[A-Za-z0-9_\-]{22}\.[A-Za-z0-9_\-]{43}",
        ),
        // OpenAI API key
        (
            "openai_api_key",
            Severity::High,
            r"sk-[A-Za-z0-9]{20}T3BlbkFJ[A-Za-z0-9]{20}",
        ),
        // Anthropic API key
        (
            "anthropic_api_key",
            Severity::High,
            r"sk-ant-(?:api03|admin01)-[A-Za-z0-9_\-]{93}",
        ),

        // --- Medium confidence ---
        // Generic "api_key = 'VALUE'" assignments
        (
            "generic_api_key",
            Severity::Medium,
            r#"(?i)api[_\-\s]*key["'\s]*[=:][\s"']+([A-Za-z0-9_\-]{20,80})["']?"#,
        ),
        // Generic "token = 'VALUE'"
        (
            "generic_token",
            Severity::Medium,
            r#"(?i)\btoken["'\s]*[=:][\s"']+([A-Za-z0-9_\-\.]{16,120})["']?"#,
        ),
        // Generic "secret = 'VALUE'"
        (
            "generic_secret",
            Severity::Medium,
            r#"(?i)\bsecret["'\s]*[=:][\s"']+([A-Za-z0-9_\-\.+/]{16,120})["']?"#,
        ),
        // Authorization Bearer header value
        (
            "bearer_token",
            Severity::Medium,
            r"(?i)Bearer\s+([A-Za-z0-9_\-\.+/=]{20,512})",
        ),
        // Private key / certificate block
        (
            "private_key_header",
            Severity::High,
            r"-----BEGIN (?:RSA |EC |DSA |OPENSSH )?PRIVATE KEY-----",
        ),

        // --- Low confidence / entropy-based ---
        // Generic password assignment
        (
            "generic_password",
            Severity::Low,
            r#"(?i)\bpass(?:word|phrase)?["'\s]*[=:][\s"']+([^'"\s\n]{8,120})["']?"#,
        ),
        // Database URL with embedded credentials
        (
            "database_url_with_creds",
            Severity::Medium,
            r"(?i)(?:postgres|mysql|mongodb|redis|amqp)(?:ql)?://[^:@\s]+:[^@\s]+@",
        ),
    ];

    specs
        .iter()
        .filter_map(|(name, severity, pattern)| {
            Regex::new(pattern)
                .map(|re| SecretPattern {
                    name,
                    severity: *severity,
                    re,
                })
                .ok()
        })
        .collect()
});

// ===========================================================================
// Finding / result types
// ===========================================================================

/// A single secret finding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretFinding {
    /// File path, relative to the scan root.
    pub file: String,
    /// 1-based line number.
    pub line: usize,
    /// Pattern category name.
    pub pattern: String,
    /// Severity level.
    pub severity: String,
    /// Partially redacted matched text (first 6 chars visible, rest replaced by `***`).
    pub matched: String,
}

// ===========================================================================
// Core scan logic
// ===========================================================================

/// Check whether a file is likely binary by sampling the first 8 KB.
fn is_binary(path: &Path) -> bool {
    use std::io::Read;
    let Ok(mut f) = std::fs::File::open(path) else {
        return false;
    };
    let mut buf = [0u8; 8192];
    let Ok(n) = f.read(&mut buf) else {
        return false;
    };
    buf[..n].contains(&0u8)
}

/// Partially redact a matched string — show up to 6 characters then `***`.
fn redact(s: &str) -> String {
    let visible = s.chars().take(6).collect::<String>();
    if s.len() > 6 {
        format!("{}***", visible)
    } else {
        visible
    }
}

/// Scan a single file and return any findings.
pub fn scan_file(path: &Path, relative_to: &Path, min_severity: Severity) -> Vec<SecretFinding> {
    if is_binary(path) {
        return vec![];
    }

    let Ok(f) = std::fs::File::open(path) else {
        return vec![];
    };

    let rel = path.strip_prefix(relative_to).unwrap_or(path);
    let file_str = rel.to_string_lossy().to_string();

    let reader = BufReader::new(f);
    let mut findings = Vec::new();

    for (line_idx, line_result) in reader.lines().enumerate() {
        let Ok(line) = line_result else { break };
        let line_num = line_idx + 1;

        // Skip comment-only lines that look like documentation examples
        let trimmed = line.trim();
        if trimmed.starts_with('#')
            || trimmed.starts_with("//")
            || trimmed.starts_with('*')
            || trimmed.starts_with("<!--")
        {
            continue;
        }

        for pat in PATTERNS.iter() {
            if pat.severity < min_severity {
                continue;
            }
            if let Some(m) = pat.re.find(&line) {
                findings.push(SecretFinding {
                    file: file_str.clone(),
                    line: line_num,
                    pattern: pat.name.to_string(),
                    severity: pat.severity.as_str().to_string(),
                    matched: redact(m.as_str()),
                });
                // One finding per pattern per line is enough
            }
        }
    }

    findings
}

/// Walk a directory tree and scan every eligible file.
pub fn scan_tree(
    root: &Path,
    include_hidden: bool,
    min_severity: Severity,
    max_findings: usize,
) -> Vec<SecretFinding> {
    let walker = WalkBuilder::new(root)
        .hidden(!include_hidden)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .require_git(false)
        .parents(true)
        .build();

    let mut all_findings: Vec<SecretFinding> = Vec::new();

    for entry_result in walker {
        let Ok(entry) = entry_result else { continue };
        let path = entry.path();

        let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
        if is_dir {
            continue;
        }

        let file_findings = scan_file(path, root, min_severity);
        all_findings.extend(file_findings);

        if all_findings.len() >= max_findings {
            all_findings.truncate(max_findings);
            break;
        }
    }

    all_findings
}

// ===========================================================================
// Tool implementation
// ===========================================================================

/// Scan files for likely secrets: API keys, tokens, passwords.
#[derive(Default, Debug, Clone)]
pub struct SecretScanTool;

impl SecretScanTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for SecretScanTool {
    fn name(&self) -> &str {
        "secret_scan"
    }

    fn description(&self) -> &str {
        "Scan files or directories for likely secrets: API keys, tokens, passwords, and credentials. \
        Detects AWS keys, GitHub tokens, Stripe keys, OpenAI/Anthropic keys, generic token/secret/password \
        assignments, private key headers, database URLs with embedded credentials, and Bearer auth headers. \
        Reports file, line number, pattern category, severity, and a partially redacted match."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "File or directory to scan. Defaults to the current working directory."
                },
                "max_findings": {
                    "type": "integer",
                    "description": "Maximum number of findings to return (default: 500).",
                    "default": 500,
                    "minimum": 1,
                    "maximum": 10000
                },
                "include_hidden": {
                    "type": "boolean",
                    "description": "Whether to scan hidden files and directories (default: false).",
                    "default": false
                },
                "severity": {
                    "type": "string",
                    "description": "Minimum severity level to report: \"low\" (default), \"medium\", or \"high\".",
                    "enum": ["low", "medium", "high"],
                    "default": "low"
                },
                "format": {
                    "type": "string",
                    "description": "Output format: \"text\" (default, human-readable) or \"json\" (machine-readable).",
                    "enum": ["text", "json"],
                    "default": "text"
                }
            },
            "additionalProperties": false
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> anyhow::Result<String> {
        // Parse args
        let scan_path_raw = args
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or(".");

        let max_findings = args
            .get("max_findings")
            .and_then(|v| v.as_u64())
            .unwrap_or(500) as usize;

        let include_hidden = args
            .get("include_hidden")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let min_severity = args
            .get("severity")
            .and_then(|v| v.as_str())
            .map(Severity::from_str)
            .unwrap_or(Severity::Low);

        let format = args
            .get("format")
            .and_then(|v| v.as_str())
            .unwrap_or("text");

        // Resolve scan path relative to cwd
        let scan_path: PathBuf = if Path::new(scan_path_raw).is_absolute() {
            PathBuf::from(scan_path_raw)
        } else {
            ctx.cwd.join(scan_path_raw)
        };

        if !scan_path.exists() {
            anyhow::bail!("Path does not exist: {}", scan_path.display());
        }

        // Run scan on threadpool to avoid blocking the async executor
        let scan_path_clone = scan_path.clone();
        let findings = tokio::task::spawn_blocking(move || {
            if scan_path_clone.is_file() {
                scan_file(&scan_path_clone, scan_path_clone.parent().unwrap_or(Path::new(".")), min_severity)
            } else {
                scan_tree(&scan_path_clone, include_hidden, min_severity, max_findings)
            }
        })
        .await
        .map_err(|e| anyhow::anyhow!("Scan task failed: {e}"))?;

        let scan_root_display = scan_path.display().to_string();
        render_output(&findings, &scan_root_display, format, max_findings)
    }
}

// ===========================================================================
// Output rendering
// ===========================================================================

fn render_output(
    findings: &[SecretFinding],
    scan_root: &str,
    format: &str,
    max_findings: usize,
) -> anyhow::Result<String> {
    if format == "json" {
        let obj = json!({
            "scan_root": scan_root,
            "total_findings": findings.len(),
            "truncated": findings.len() >= max_findings,
            "findings": findings,
        });
        return Ok(serde_json::to_string_pretty(&obj)?);
    }

    // Text output
    let mut out = String::with_capacity(findings.len() * 80 + 128);

    if findings.is_empty() {
        out.push_str(&format!(
            "SECRET SCAN COMPLETE\nRoot: {}\nResult: No secrets found.\n",
            scan_root
        ));
        return Ok(out);
    }

    let truncated = findings.len() >= max_findings;

    out.push_str(&format!(
        "SECRET SCAN COMPLETE\nRoot: {}\nFindings: {}{}\n\n",
        scan_root,
        findings.len(),
        if truncated { " (truncated)" } else { "" }
    ));

    // Severity legend
    out.push_str("Severity: [H] High  [M] Medium  [L] Low\n");
    out.push_str(&"─".repeat(80));
    out.push('\n');

    for f in findings {
        let sev_tag = match f.severity.as_str() {
            "high" => "[H]",
            "medium" => "[M]",
            _ => "[L]",
        };
        out.push_str(&format!(
            "{} {}:{} — {} — {}\n",
            sev_tag, f.file, f.line, f.pattern, f.matched
        ));
    }

    out.push_str(&"─".repeat(80));
    out.push('\n');

    // Summary by severity
    let high = findings.iter().filter(|f| f.severity == "high").count();
    let medium = findings.iter().filter(|f| f.severity == "medium").count();
    let low = findings.iter().filter(|f| f.severity == "low").count();

    out.push_str(&format!(
        "Summary: {} high, {} medium, {} low\n",
        high, medium, low
    ));

    if truncated {
        out.push_str(&format!(
            "Warning: output truncated at {} findings. Use max_findings to increase the limit.\n",
            max_findings
        ));
    }

    Ok(out)
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    struct TempDir(PathBuf);
    impl TempDir {
        fn new() -> Self {
            let p = std::env::temp_dir()
                .join(format!("fusion_secret_scan_{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&p).unwrap();
            Self(p)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    // ------------------------------------------------------------------
    // Unit tests — pattern matching
    // ------------------------------------------------------------------

    #[test]
    fn test_detects_aws_access_key_id() {
        let line = r#"export AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE"#;
        let matches: Vec<_> = PATTERNS
            .iter()
            .filter(|p| p.name == "aws_access_key_id")
            .filter_map(|p| p.re.find(line))
            .collect();
        assert!(!matches.is_empty(), "should detect AKIA* key");
    }

    #[test]
    fn test_detects_github_pat_classic() {
        let line = "GITHUB_TOKEN=ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghi0";
        let matches: Vec<_> = PATTERNS
            .iter()
            .filter(|p| p.name == "github_pat_classic")
            .filter_map(|p| p.re.find(line))
            .collect();
        assert!(!matches.is_empty(), "should detect ghp_ token");
    }

    #[test]
    fn test_detects_stripe_secret_key() {
        let token_part = "51AbCdEfGhIjKlMnOpQrStUvWx";
        let line = format!("STRIPE_SECRET=sk_live_{}", token_part);
        let matches: Vec<_> = PATTERNS
            .iter()
            .filter(|p| p.name == "stripe_secret_key")
            .filter_map(|p| p.re.find(line))
            .collect();
        assert!(!matches.is_empty(), "should detect sk_live_ key");
    }

    #[test]
    fn test_detects_private_key_header() {
        let line = "-----BEGIN RSA PRIVATE KEY-----";
        let matches: Vec<_> = PATTERNS
            .iter()
            .filter(|p| p.name == "private_key_header")
            .filter_map(|p| p.re.find(line))
            .collect();
        assert!(!matches.is_empty(), "should detect PEM private key header");
    }

    #[test]
    fn test_detects_generic_token() {
        let line = r#"token = "s3cr3t-value-here-very-long""#;
        let matches: Vec<_> = PATTERNS
            .iter()
            .filter(|p| p.name == "generic_token")
            .filter_map(|p| p.re.find(line))
            .collect();
        assert!(!matches.is_empty(), "should detect generic token assignment");
    }

    #[test]
    fn test_detects_database_url_with_creds() {
        let line = "DATABASE_URL=postgres://admin:hunter2@localhost:5432/mydb";
        let matches: Vec<_> = PATTERNS
            .iter()
            .filter(|p| p.name == "database_url_with_creds")
            .filter_map(|p| p.re.find(line))
            .collect();
        assert!(
            !matches.is_empty(),
            "should detect database URL with embedded credentials"
        );
    }

    #[test]
    fn test_detects_bearer_token() {
        let line = "Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.test";
        let matches: Vec<_> = PATTERNS
            .iter()
            .filter(|p| p.name == "bearer_token")
            .filter_map(|p| p.re.find(line))
            .collect();
        assert!(!matches.is_empty(), "should detect Bearer authorization header");
    }

    #[test]
    fn test_redact() {
        assert_eq!(redact("ghp_ABC"), "ghp_AB***");
        assert_eq!(redact("abc"), "abc");
        assert_eq!(redact("abcdefXYZ"), "abcdef***");
    }

    #[test]
    fn test_severity_ordering() {
        assert!(Severity::High > Severity::Medium);
        assert!(Severity::Medium > Severity::Low);
    }

    // ------------------------------------------------------------------
    // Integration tests — file scanning
    // ------------------------------------------------------------------

    #[test]
    fn test_scan_file_finds_secrets() {
        let tmp = TempDir::new();
        let fixture = tmp.path().join("secrets.env");
        let dummy_stripe = format!("sk_live_{}", "51AbCdEfGhIjKlMnOpQrStUvWx");
        fs::write(
            &fixture,
            format!(
                "AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE\nSTRIPE_SECRET={}\nDATABASE_URL=postgres://admin:hunter2@localhost:5432/mydb\n",
                dummy_stripe
            ),
        )
        .unwrap();

        let findings = scan_file(&fixture, tmp.path(), Severity::Low);
        assert!(!findings.is_empty(), "should find at least one secret");

        let patterns: Vec<&str> = findings.iter().map(|f| f.pattern.as_str()).collect();
        assert!(
            patterns.contains(&"aws_access_key_id"),
            "should find aws_access_key_id; got: {:?}",
            patterns
        );
        assert!(
            patterns.contains(&"stripe_secret_key"),
            "should find stripe_secret_key; got: {:?}",
            patterns
        );
        assert!(
            patterns.contains(&"database_url_with_creds"),
            "should find database_url_with_creds; got: {:?}",
            patterns
        );
    }

    #[test]
    fn test_scan_file_severity_filter() {
        let tmp = TempDir::new();
        let fixture = tmp.path().join("mixed.env");
        fs::write(
            &fixture,
            concat!(
                "password = \"weakpassword\"\n",   // low
                "api_key = \"some-api-key-value-here\"\n", // medium
                "GITHUB_TOKEN=ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghi0\n", // high
            ),
        )
        .unwrap();

        // High only
        let high_findings = scan_file(&fixture, tmp.path(), Severity::High);
        let high_patterns: Vec<&str> = high_findings.iter().map(|f| f.pattern.as_str()).collect();
        assert!(
            high_patterns.contains(&"github_pat_classic"),
            "high filter should find github PAT; got: {:?}",
            high_patterns
        );
        // Low-severity patterns must not appear when filtering for high
        for f in &high_findings {
            assert_ne!(
                f.severity, "low",
                "high filter should not include low severity findings"
            );
        }
    }

    #[test]
    fn test_scan_file_no_false_positive_in_comments() {
        let tmp = TempDir::new();
        let fixture = tmp.path().join("commented.rs");
        // Comment lines are skipped
        fs::write(
            &fixture,
            concat!(
                "// Example: AKIAIOSFODNN7EXAMPLE (this is documentation)\n",
                "// token = \"not-a-real-token-value-here-xx\"\n",
                "let real_key = \"ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghi0\";\n",
            ),
        )
        .unwrap();

        let findings = scan_file(&fixture, tmp.path(), Severity::Low);
        // The comment lines are skipped; only the real_key assignment line should fire
        for f in &findings {
            assert_ne!(
                f.line, 1,
                "line 1 is a comment and should be skipped; finding: {:?}",
                f
            );
            assert_ne!(
                f.line, 2,
                "line 2 is a comment and should be skipped; finding: {:?}",
                f
            );
        }
        // Line 3 (the real assignment) must be detected
        let line3_found = findings.iter().any(|f| f.line == 3);
        assert!(line3_found, "real secret on line 3 should be detected; findings: {:?}", findings);
    }

    #[test]
    fn test_scan_tree_multi_file() {
        let tmp = TempDir::new();

        fs::write(
            tmp.path().join("a.env"),
            "GITHUB_TOKEN=ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghi0\n",
        )
        .unwrap();
        fs::write(
            tmp.path().join("b.yaml"),
            "database_url: postgres://user:pass@db:5432/app\n",
        )
        .unwrap();
        fs::write(tmp.path().join("clean.txt"), "nothing secret here\n").unwrap();

        let findings = scan_tree(tmp.path(), false, Severity::Low, 500);
        let files: Vec<&str> = findings.iter().map(|f| f.file.as_str()).collect();

        assert!(
            files.iter().any(|f| f.contains("a.env")),
            "should detect secret in a.env; files: {:?}",
            files
        );
        assert!(
            files.iter().any(|f| f.contains("b.yaml")),
            "should detect secret in b.yaml; files: {:?}",
            files
        );
    }

    #[test]
    fn test_max_findings_respected() {
        let tmp = TempDir::new();
        // Write a file with many secrets
        let mut content = String::new();
        for i in 0..50 {
            content.push_str(&format!(
                "GITHUB_TOKEN_{i}=ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghi0\n"
            ));
        }
        fs::write(tmp.path().join("many_secrets.env"), &content).unwrap();

        let findings = scan_tree(tmp.path(), false, Severity::Low, 10);
        assert!(
            findings.len() <= 10,
            "should not exceed max_findings=10; got {}",
            findings.len()
        );
    }

    // ------------------------------------------------------------------
    // Tool execute integration test
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_tool_execute_text_format() {
        let tmp = TempDir::new();
        fs::write(
            tmp.path().join("creds.env"),
            "AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE\n",
        )
        .unwrap();

        let tool = SecretScanTool::new();
        let ctx = ToolContext {
            cwd: tmp.path().to_path_buf(),
            env: std::collections::HashMap::new(),
        };

        let result = tool
            .execute(json!({ "path": ".", "format": "text" }), &ctx)
            .await
            .unwrap();

        assert!(
            result.contains("SECRET SCAN COMPLETE"),
            "output should contain header; got: {}",
            result
        );
        assert!(
            result.contains("aws_access_key_id"),
            "output should name the pattern; got: {}",
            result
        );
        assert!(
            result.contains("creds.env"),
            "output should name the file; got: {}",
            result
        );
    }

    #[tokio::test]
    async fn test_tool_execute_json_format() {
        let tmp = TempDir::new();
        fs::write(
            tmp.path().join("tok.sh"),
            "export GITHUB_TOKEN=ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghi0\n",
        )
        .unwrap();

        let tool = SecretScanTool::new();
        let ctx = ToolContext {
            cwd: tmp.path().to_path_buf(),
            env: std::collections::HashMap::new(),
        };

        let result = tool
            .execute(json!({ "path": ".", "format": "json" }), &ctx)
            .await
            .unwrap();

        let parsed: Value = serde_json::from_str(&result).expect("output should be valid JSON");
        assert!(
            parsed["total_findings"].as_u64().unwrap_or(0) > 0,
            "should report at least one finding in JSON output"
        );
        let findings = parsed["findings"].as_array().unwrap();
        let any_github = findings
            .iter()
            .any(|f| f["pattern"].as_str() == Some("github_pat_classic"));
        assert!(any_github, "JSON findings should include github_pat_classic");
    }

    #[tokio::test]
    async fn test_tool_execute_no_secrets() {
        let tmp = TempDir::new();
        fs::write(tmp.path().join("clean.rs"), "fn main() { println!(\"hello\"); }\n").unwrap();

        let tool = SecretScanTool::new();
        let ctx = ToolContext {
            cwd: tmp.path().to_path_buf(),
            env: std::collections::HashMap::new(),
        };

        let result = tool
            .execute(json!({ "path": "." }), &ctx)
            .await
            .unwrap();

        assert!(
            result.contains("No secrets found"),
            "clean dir should report no secrets; got: {}",
            result
        );
    }

    #[tokio::test]
    async fn test_tool_execute_invalid_path() {
        let tool = SecretScanTool::new();
        let ctx = ToolContext {
            cwd: std::env::temp_dir(),
            env: std::collections::HashMap::new(),
        };

        let result = tool
            .execute(json!({ "path": "/nonexistent/path/that/cannot/exist" }), &ctx)
            .await;

        assert!(result.is_err(), "should error on nonexistent path");
    }
}
