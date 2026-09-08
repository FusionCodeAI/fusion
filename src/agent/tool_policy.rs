//! Tool policy engine and security decision evaluation.
//!
//! Provides granular access control and safety evaluation for tool executions:
//! - Destructive bash commands (`rm -rf`, `drop table`, `format`, `mkfs`) require user confirmation (`Ask`) or are blocked (`Deny`).
//! - Sensitive file access (`.env`, `id_rsa`, `credentials`, `secrets`) requires user confirmation (`Ask`).
//! - Safe read-only tools (`read`, `grep`, `glob`, `lsp`) are automatically permitted (`Allow`).
//! - Supports loading and applying custom policy rules from `.fusion/policy.toml`.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ============================================================================
// Errors
// ============================================================================

/// Errors emitted during policy parsing, loading, or configuration.
#[derive(thiserror::Error, Debug, Clone, PartialEq, Eq)]
pub enum PolicyError {
    #[error("IO error while accessing policy file '{0}': {1}")]
    Io(String, String),

    #[error("Failed to parse policy TOML: {0}")]
    Parse(String),

    #[error("Invalid policy configuration: {0}")]
    Config(String),
}

// ============================================================================
// Policy Decision
// ============================================================================

/// The decision resulting from evaluating a tool execution request against active policies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PolicyDecision {
    /// Tool execution is allowed automatically without user intervention.
    Allow,
    /// Tool execution requires explicit user confirmation before proceeding.
    Ask(String),
    /// Tool execution is denied and strictly prohibited.
    Deny(String),
}

impl PolicyDecision {
    /// Returns `true` if the decision allows automatic execution.
    pub fn is_allow(&self) -> bool {
        matches!(self, Self::Allow)
    }

    /// Returns `true` if the decision requires asking the user for confirmation.
    pub fn is_ask(&self) -> bool {
        matches!(self, Self::Ask(_))
    }

    /// Returns `true` if the decision denies execution.
    pub fn is_deny(&self) -> bool {
        matches!(self, Self::Deny(_))
    }

    /// Returns the reason or message associated with `Ask` or `Deny`, if present.
    pub fn message(&self) -> Option<&str> {
        match self {
            Self::Allow => None,
            Self::Ask(msg) | Self::Deny(msg) => Some(msg.as_str()),
        }
    }

    /// Alias for `message`.
    pub fn reason(&self) -> Option<&str> {
        self.message()
    }
}

impl fmt::Display for PolicyDecision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Allow => write!(f, "Allow"),
            Self::Ask(reason) => write!(f, "Ask: {}", reason),
            Self::Deny(reason) => write!(f, "Deny: {}", reason),
        }
    }
}

// ============================================================================
// Rule Action & Policy Rules
// ============================================================================

/// Action specified by a policy rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum RuleAction {
    #[default]
    Allow,
    Ask,
    Deny,
}

impl RuleAction {
    /// Parses loose string representations into `RuleAction`.
    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "allow" | "permitted" | "ok" => Some(Self::Allow),
            "ask" | "confirm" | "prompt" => Some(Self::Ask),
            "deny" | "block" | "forbidden" | "disallow" => Some(Self::Deny),
            _ => None,
        }
    }

    /// Converts action into a `PolicyDecision`.
    pub fn to_decision(&self, reason: impl Into<String>) -> PolicyDecision {
        match self {
            Self::Allow => PolicyDecision::Allow,
            Self::Ask => PolicyDecision::Ask(reason.into()),
            Self::Deny => PolicyDecision::Deny(reason.into()),
        }
    }
}

/// A specific rule matching tool names, command patterns, or file path patterns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyRule {
    /// Optional tool name filter (e.g. `"bash"`, `"write"`, `"*"` for any tool).
    pub tool: Option<String>,
    /// Optional regex or substring pattern to match against commands or arguments.
    pub pattern: Option<String>,
    /// Optional path pattern (e.g. `"*.env"`, `"id_rsa"`, `"secrets/*"`).
    pub path_pattern: Option<String>,
    /// The action to take when this rule matches.
    pub action: RuleAction,
    /// Explanation provided when this rule triggers.
    pub reason: Option<String>,
}

impl PolicyRule {
    /// Evaluates if this rule matches the given tool call.
    pub fn matches(&self, tool_name: &str, args: &Value) -> bool {
        // 1. Tool name match
        if let Some(t) = &self.tool {
            if t != "*" && !t.eq_ignore_ascii_case(tool_name) {
                return false;
            }
        }

        // 2. Pattern match (command / text)
        if let Some(pat) = &self.pattern {
            if !matches_command_pattern(pat, args) {
                return false;
            }
        }

        // 3. Path pattern match
        if let Some(path_pat) = &self.path_pattern {
            if !matches_path_pattern(path_pat, args) {
                return false;
            }
        }

        true
    }

    /// Converts this rule's outcome into a `PolicyDecision`.
    pub fn to_decision(&self) -> PolicyDecision {
        let default_reason = match &self.tool {
            Some(t) => format!("Matched custom policy rule for tool '{t}'"),
            None => "Matched custom policy rule".to_string(),
        };
        let reason = self.reason.clone().unwrap_or(default_reason);
        self.action.to_decision(reason)
    }
}

// ============================================================================
// TOML Configuration Data Structures
// ============================================================================

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct PolicyConfigFile {
    #[serde(default)]
    default_decision: Option<String>,
    #[serde(default)]
    default_action: Option<String>,

    #[serde(default)]
    rules: Vec<PolicyRuleConfig>,

    #[serde(default)]
    bash: Option<BashPolicyConfig>,

    #[serde(default)]
    sensitive_files: Option<SensitiveFilesConfig>,

    #[serde(default)]
    tools: Option<ToolsPolicyConfig>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct PolicyRuleConfig {
    tool: Option<String>,
    pattern: Option<String>,
    path_pattern: Option<String>,
    action: Option<String>,
    reason: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct BashPolicyConfig {
    #[serde(default)]
    ask_patterns: Vec<String>,
    #[serde(default)]
    deny_patterns: Vec<String>,
    #[serde(default)]
    allow_patterns: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct SensitiveFilesConfig {
    #[serde(default)]
    patterns: Vec<String>,
    action: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ToolsPolicyConfig {
    #[serde(default)]
    allow: Vec<String>,
    #[serde(default)]
    ask: Vec<String>,
    #[serde(default)]
    deny: Vec<String>,
}

// ============================================================================
// Built-in Lightweight TOML Parser
// ============================================================================

/// Parses a TOML string into a `serde_json::Value` structure without external dependencies.
pub fn parse_toml_to_value(input: &str) -> Result<Value, PolicyError> {
    let mut root = serde_json::Map::new();
    let lines: Vec<&str> = input.lines().collect();
    let mut i = 0;

    let mut is_array_target = false;
    let mut current_path: Vec<String> = Vec::new();

    while i < lines.len() {
        let raw_line = lines[i].trim();
        i += 1;

        if raw_line.is_empty() || raw_line.starts_with('#') {
            continue;
        }

        // Strip comments not enclosed in quotes
        let mut clean_line = String::new();
        let mut in_str = false;
        let mut str_ch = '"';
        let mut escape = false;

        for ch in raw_line.chars() {
            if in_str {
                clean_line.push(ch);
                if escape {
                    escape = false;
                } else if ch == '\\' {
                    escape = true;
                } else if ch == str_ch {
                    in_str = false;
                }
            } else if ch == '"' || ch == '\'' {
                in_str = true;
                str_ch = ch;
                clean_line.push(ch);
            } else if ch == '#' {
                break;
            } else {
                clean_line.push(ch);
            }
        }

        let line = clean_line.trim();
        if line.is_empty() {
            continue;
        }

        // Table array header: [[section.sub]]
        if line.starts_with("[[") && line.ends_with("]]") && line.len() >= 4 {
            let path_str = line[2..line.len() - 2].trim();
            current_path = path_str
                .split('.')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            is_array_target = true;

            if current_path.is_empty() {
                continue;
            }

            let mut curr = &mut root;
            for seg in &current_path[..current_path.len() - 1] {
                curr = curr
                    .entry(seg.clone())
                    .or_insert_with(|| Value::Object(serde_json::Map::new()))
                    .as_object_mut()
                    .ok_or_else(|| {
                        PolicyError::Parse(format!("Expected object table at '{}'", seg))
                    })?;
            }
            let last_seg = current_path.last().unwrap();
            let arr = curr
                .entry(last_seg.clone())
                .or_insert_with(|| Value::Array(Vec::new()))
                .as_array_mut()
                .ok_or_else(|| PolicyError::Parse(format!("Expected array at '{}'", last_seg)))?;
            arr.push(Value::Object(serde_json::Map::new()));
            continue;
        }

        // Table header: [section.sub]
        if line.starts_with('[') && line.ends_with(']') && line.len() >= 2 {
            let path_str = line[1..line.len() - 1].trim();
            current_path = path_str
                .split('.')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            is_array_target = false;

            let mut curr = &mut root;
            for seg in &current_path {
                curr = curr
                    .entry(seg.clone())
                    .or_insert_with(|| Value::Object(serde_json::Map::new()))
                    .as_object_mut()
                    .ok_or_else(|| {
                        PolicyError::Parse(format!("Expected object table at '{}'", seg))
                    })?;
            }
            continue;
        }

        // Key = Value pair
        if let Some(eq_idx) = line.find('=') {
            let key = line[..eq_idx].trim().to_string();
            let mut val_str = line[eq_idx + 1..].trim().to_string();

            // Handle multi-line arrays or inline objects
            if (val_str.starts_with('[') && !val_str.ends_with(']'))
                || (val_str.starts_with('{') && !val_str.ends_with('}'))
            {
                while i < lines.len() {
                    let next_line = lines[i].trim();
                    i += 1;
                    let no_comment = next_line.split('#').next().unwrap_or("").trim();
                    val_str.push(' ');
                    val_str.push_str(no_comment);
                    if (val_str.starts_with('[') && val_str.ends_with(']'))
                        || (val_str.starts_with('{') && val_str.ends_with('}'))
                    {
                        break;
                    }
                }
            }

            let parsed_val = parse_toml_value(&val_str);

            if is_array_target {
                let mut curr = &mut root;
                for seg in &current_path[..current_path.len() - 1] {
                    curr = curr
                        .get_mut(seg)
                        .and_then(|v| v.as_object_mut())
                        .ok_or_else(|| {
                            PolicyError::Parse(format!(
                                "Failed traversing to array target segment '{}'",
                                seg
                            ))
                        })?;
                }
                let last_seg = current_path.last().unwrap();
                let arr = curr
                    .get_mut(last_seg)
                    .and_then(|v| v.as_array_mut())
                    .ok_or_else(|| {
                        PolicyError::Parse(format!("Expected array at '{}'", last_seg))
                    })?;
                if let Some(last_obj) = arr.last_mut().and_then(|v| v.as_object_mut()) {
                    insert_dotted_key(last_obj, &key, parsed_val);
                }
            } else if current_path.is_empty() {
                insert_dotted_key(&mut root, &key, parsed_val);
            } else {
                let mut curr = &mut root;
                for seg in &current_path {
                    curr = curr
                        .entry(seg.clone())
                        .or_insert_with(|| Value::Object(serde_json::Map::new()))
                        .as_object_mut()
                        .ok_or_else(|| {
                            PolicyError::Parse(format!("Expected table segment '{}'", seg))
                        })?;
                }
                insert_dotted_key(curr, &key, parsed_val);
            }
        }
    }

    Ok(Value::Object(root))
}

fn insert_dotted_key(map: &mut serde_json::Map<String, Value>, key: &str, val: Value) {
    let parts: Vec<&str> = key
        .split('.')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    if parts.len() <= 1 {
        map.insert(key.to_string(), val);
    } else {
        let mut curr = map;
        for part in &parts[..parts.len() - 1] {
            curr = curr
                .entry(part.to_string())
                .or_insert_with(|| Value::Object(serde_json::Map::new()))
                .as_object_mut()
                .unwrap();
        }
        curr.insert(parts.last().unwrap().to_string(), val);
    }
}

fn parse_toml_value(raw: &str) -> Value {
    let s = raw.trim();
    if s.is_empty() {
        return Value::String(String::new());
    }

    // Quoted strings
    if (s.starts_with('"') && s.ends_with('"') && s.len() >= 2)
        || (s.starts_with('\'') && s.ends_with('\'') && s.len() >= 2)
    {
        let inner = &s[1..s.len() - 1];
        let unescaped = inner
            .replace("\\n", "\n")
            .replace("\\t", "\t")
            .replace("\\r", "\r")
            .replace("\\\"", "\"")
            .replace("\\\\", "\\");
        return Value::String(unescaped);
    }

    // Booleans
    if s.eq_ignore_ascii_case("true") {
        return Value::Bool(true);
    }
    if s.eq_ignore_ascii_case("false") {
        return Value::Bool(false);
    }

    // Arrays: [ ... ]
    if s.starts_with('[') && s.ends_with(']') {
        let inner = s[1..s.len() - 1].trim();
        let mut items = Vec::new();
        let mut curr = String::new();
        let mut in_str = false;
        let mut str_ch = '"';
        let mut escape = false;

        for ch in inner.chars() {
            if in_str {
                curr.push(ch);
                if escape {
                    escape = false;
                } else if ch == '\\' {
                    escape = true;
                } else if ch == str_ch {
                    in_str = false;
                }
            } else if ch == '"' || ch == '\'' {
                in_str = true;
                str_ch = ch;
                curr.push(ch);
            } else if ch == ',' {
                let trimmed = curr.trim();
                if !trimmed.is_empty() {
                    items.push(parse_toml_value(trimmed));
                }
                curr.clear();
            } else {
                curr.push(ch);
            }
        }
        let trimmed = curr.trim();
        if !trimmed.is_empty() {
            items.push(parse_toml_value(trimmed));
        }
        return Value::Array(items);
    }

    // Inline tables: { ... }
    if s.starts_with('{') && s.ends_with('}') {
        let inner = s[1..s.len() - 1].trim();
        let mut map = serde_json::Map::new();
        for pair in inner.split(',') {
            let pair = pair.trim();
            if let Some((k, v)) = pair.split_once('=') {
                map.insert(k.trim().to_string(), parse_toml_value(v.trim()));
            }
        }
        return Value::Object(map);
    }

    // Integers
    if let Ok(num) = s.parse::<i64>() {
        return Value::Number(num.into());
    }

    // Floats
    if let Ok(num) = s.parse::<f64>() {
        if let Some(n) = serde_json::Number::from_f64(num) {
            return Value::Number(n);
        }
    }

    // Fallback: unquoted bare words as strings (e.g. action = allow)
    Value::String(s.to_string())
}

// ============================================================================
// Tool Policy Engine
// ============================================================================

/// Policy evaluation engine that verifies tool executions against built-in and custom policies.
#[derive(Debug, Clone)]
pub struct ToolPolicyEngine {
    /// Custom rules loaded from `.fusion/policy.toml` or configured programmatically.
    pub custom_rules: Vec<PolicyRule>,
    /// Default decision for unclassified tools (defaults to `PolicyDecision::Allow`).
    pub default_decision: PolicyDecision,
    /// Sensitive file patterns that prompt for confirmation when accessed.
    pub sensitive_patterns: Vec<String>,
    /// Action to take on sensitive file access (defaults to `RuleAction::Ask`).
    pub sensitive_files_action: RuleAction,
    /// Tools deemed safe and read-only by default.
    pub safe_read_only_tools: HashSet<String>,
    /// Per-tool default actions from configuration.
    pub custom_tool_actions: HashMap<String, RuleAction>,
}

impl Default for ToolPolicyEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolPolicyEngine {
    /// Creates a new `ToolPolicyEngine` with default built-in safety rules.
    pub fn new() -> Self {
        let mut safe_read_only_tools = HashSet::new();
        for tool in [
            "read",
            "file_read",
            "grep",
            "glob",
            "lsp",
            "symbols",
            "syntax",
            "search",
        ] {
            safe_read_only_tools.insert(tool.to_string());
        }

        Self {
            custom_rules: Vec::new(),
            default_decision: PolicyDecision::Allow,
            sensitive_patterns: vec![
                ".env".to_string(),
                "id_rsa".to_string(),
                "credentials".to_string(),
                "secrets".to_string(),
            ],
            sensitive_files_action: RuleAction::Ask,
            safe_read_only_tools,
            custom_tool_actions: HashMap::new(),
        }
    }

    /// Loads custom rules from `.fusion/policy.toml` located in `workspace_dir`.
    ///
    /// If the policy file does not exist, returns a standard engine with built-in rules.
    pub fn load_from_dir<P: AsRef<Path>>(workspace_dir: P) -> Self {
        Self::try_load_from_dir(workspace_dir).unwrap_or_else(|_| Self::new())
    }

    /// Attempts to load custom rules from `.fusion/policy.toml` or `policy.toml` in `dir`.
    pub fn try_load_from_dir<P: AsRef<Path>>(workspace_dir: P) -> Result<Self, PolicyError> {
        let dir = workspace_dir.as_ref();
        let candidates = [
            dir.join(".fusion").join("policy.toml"),
            dir.join("policy.toml"),
        ];

        for candidate in &candidates {
            if candidate.exists() {
                return Self::load_from_file(candidate);
            }
        }

        Ok(Self::new())
    }

    /// Loads custom rules from an explicit file path.
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self, PolicyError> {
        let path = path.as_ref();
        let content = std::fs::read_to_string(path)
            .map_err(|e| PolicyError::Io(path.display().to_string(), e.to_string()))?;
        Self::from_toml(&content)
    }

    /// Parses policy rules and configuration from a TOML string.
    pub fn from_toml(content: &str) -> Result<Self, PolicyError> {
        let val = parse_toml_to_value(content)?;
        let config: PolicyConfigFile = serde_json::from_value(val).map_err(|e| {
            PolicyError::Config(format!("Failed to deserialize policy config: {e}"))
        })?;

        let mut engine = Self::new();

        // 1. Default action / decision
        if let Some(def) = config
            .default_decision
            .as_ref()
            .or(config.default_action.as_ref())
        {
            if let Some(action) = RuleAction::from_str_loose(def) {
                engine.default_decision = action.to_decision("Default policy decision");
            }
        }

        // 2. Custom rules
        for rule_cfg in config.rules {
            let action = rule_cfg
                .action
                .as_deref()
                .and_then(RuleAction::from_str_loose)
                .unwrap_or(RuleAction::Ask);

            engine.custom_rules.push(PolicyRule {
                tool: rule_cfg.tool,
                pattern: rule_cfg.pattern,
                path_pattern: rule_cfg.path_pattern,
                action,
                reason: rule_cfg.reason,
            });
        }

        // 3. Bash section
        if let Some(bash) = config.bash {
            for pat in bash.deny_patterns {
                engine.custom_rules.push(PolicyRule {
                    tool: Some("bash".to_string()),
                    pattern: Some(pat),
                    path_pattern: None,
                    action: RuleAction::Deny,
                    reason: Some("Blocked by bash deny policy".to_string()),
                });
            }
            for pat in bash.ask_patterns {
                engine.custom_rules.push(PolicyRule {
                    tool: Some("bash".to_string()),
                    pattern: Some(pat),
                    path_pattern: None,
                    action: RuleAction::Ask,
                    reason: Some("Requires confirmation by bash ask policy".to_string()),
                });
            }
            for pat in bash.allow_patterns {
                engine.custom_rules.push(PolicyRule {
                    tool: Some("bash".to_string()),
                    pattern: Some(pat),
                    path_pattern: None,
                    action: RuleAction::Allow,
                    reason: Some("Explicitly allowed by bash policy".to_string()),
                });
            }
        }

        // 4. Sensitive files section
        if let Some(sensitive) = config.sensitive_files {
            for pat in sensitive.patterns {
                if !engine.sensitive_patterns.contains(&pat) {
                    engine.sensitive_patterns.push(pat);
                }
            }
            if let Some(act_str) = &sensitive.action {
                if let Some(action) = RuleAction::from_str_loose(act_str) {
                    engine.sensitive_files_action = action;
                }
            }
        }

        // 5. Tools section
        if let Some(tools) = config.tools {
            for t in tools.allow {
                engine
                    .custom_tool_actions
                    .insert(t.clone(), RuleAction::Allow);
                engine.safe_read_only_tools.insert(t);
            }
            for t in tools.ask {
                engine.custom_tool_actions.insert(t, RuleAction::Ask);
            }
            for t in tools.deny {
                engine.custom_tool_actions.insert(t, RuleAction::Deny);
            }
        }

        Ok(engine)
    }

    /// Builder helper to set the default decision.
    pub fn with_default_decision(mut self, decision: PolicyDecision) -> Self {
        self.default_decision = decision;
        self
    }

    /// Adds a custom policy rule to the engine.
    pub fn add_rule(&mut self, rule: PolicyRule) {
        self.custom_rules.push(rule);
    }

    /// Builder helper to add a custom policy rule.
    pub fn with_rule(mut self, rule: PolicyRule) -> Self {
        self.add_rule(rule);
        self
    }

    /// Adds an additional sensitive file pattern.
    pub fn add_sensitive_pattern<S: Into<String>>(&mut self, pattern: S) {
        self.sensitive_patterns.push(pattern.into());
    }

    /// Adds an additional tool to the safe read-only allowlist.
    pub fn add_safe_tool<S: Into<String>>(&mut self, tool: S) {
        self.safe_read_only_tools.insert(tool.into());
    }

    /// Evaluates a tool execution request against active policies.
    ///
    /// # Rules Evaluation Precedence:
    /// 1. Custom rules loaded from `.fusion/policy.toml` or added programmatically.
    /// 2. Per-tool configuration from the `[tools]` section.
    /// 3. Sensitive file access (`.env`, `id_rsa`, `credentials`, `secrets`) -> `Ask(...)`.
    /// 4. Destructive bash commands (`rm -rf`, `drop table`, `format`, `mkfs`) -> `Ask(...)` or `Deny(...)`.
    /// 5. Safe read-only tools (`read`, `grep`, `glob`, `lsp`) -> `Allow`.
    /// 6. Default policy decision (normally `Allow`).
    pub fn evaluate(&self, tool_name: &str, args: &Value) -> PolicyDecision {
        // 1. Evaluate custom rules
        for rule in &self.custom_rules {
            if rule.matches(tool_name, args) {
                return rule.to_decision();
            }
        }

        // 2. Evaluate per-tool explicit configured action
        if let Some(action) = self.custom_tool_actions.get(tool_name) {
            let reason = match action {
                RuleAction::Allow => "Tool allowed by configuration",
                RuleAction::Ask => "Tool requires confirmation by configuration",
                RuleAction::Deny => "Tool is denied by configuration",
            };
            return action.to_decision(reason);
        }

        // 3. Sensitive file access check
        if let Some(sensitive_pattern) = self.find_sensitive_file(tool_name, args) {
            let msg =
                format!("Access to sensitive file or pattern detected: '{sensitive_pattern}'");
            return self.sensitive_files_action.to_decision(msg);
        }

        // 4. Destructive bash command check
        if self.is_bash_tool(tool_name) {
            if let Some(decision) = self.evaluate_bash_command(args) {
                return decision;
            }
        }

        // 5. Safe read-only tools
        if self.is_safe_read_only_tool(tool_name) {
            return PolicyDecision::Allow;
        }

        // 6. Default decision
        self.default_decision.clone()
    }

    /// Checks if a tool name refers to a shell/command execution tool.
    pub fn is_bash_tool(&self, tool_name: &str) -> bool {
        matches!(
            tool_name.to_ascii_lowercase().as_str(),
            "bash" | "sh" | "shell" | "exec" | "terminal" | "cmd" | "eval" | "run_command"
        )
    }

    /// Checks if a tool is recognized as a safe read-only tool.
    pub fn is_safe_read_only_tool(&self, tool_name: &str) -> bool {
        self.safe_read_only_tools
            .contains(&tool_name.to_ascii_lowercase())
    }

    /// Scans arguments and command text for references to sensitive files or secrets.
    pub fn find_sensitive_file(&self, tool_name: &str, args: &Value) -> Option<String> {
        // 1. Inspect extracted path arguments
        for path in extract_all_paths(args) {
            if let Some(matched) = self.matches_sensitive_path(&path) {
                return Some(matched);
            }
        }

        // 2. If command tool, inspect command string tokens
        if self.is_bash_tool(tool_name) {
            if let Some(cmd) = extract_command_str(args) {
                for token in cmd.split_whitespace() {
                    let cleaned = token.trim_matches(|c| {
                        c == '\''
                            || c == '"'
                            || c == ';'
                            || c == '>'
                            || c == '<'
                            || c == '|'
                            || c == '`'
                    });
                    if let Some(matched) = self.matches_sensitive_path(cleaned) {
                        return Some(matched);
                    }
                }
            }
        }

        None
    }

    fn matches_sensitive_path(&self, path_str: &str) -> Option<String> {
        let lower = path_str.to_ascii_lowercase().replace('\\', "/");
        let filename = lower.rsplit('/').next().unwrap_or(&lower);

        // Check custom patterns first
        for pat in &self.sensitive_patterns {
            let pat_lower = pat.to_ascii_lowercase();
            if filename == pat_lower || filename.contains(&pat_lower) || lower.contains(&pat_lower)
            {
                return Some(pat.clone());
            }
        }

        // Built-in sensitive pattern checks:
        // 1. .env files
        if filename == ".env"
            || filename.starts_with(".env.")
            || filename.ends_with(".env")
            || lower.contains("/.env")
        {
            return Some(".env".to_string());
        }

        // 2. id_rsa and SSH private keys
        if filename.contains("id_rsa")
            || filename.contains("id_ed25519")
            || filename.contains("id_ecdsa")
            || filename.contains("id_dsa")
            || lower.contains("/.ssh/")
        {
            return Some("id_rsa".to_string());
        }

        // 3. Credentials
        if filename.contains("credentials") || lower.contains("/credentials") {
            return Some("credentials".to_string());
        }

        // 4. Secrets
        if filename.contains("secrets")
            || filename.starts_with("secret.")
            || filename.ends_with(".secret")
            || filename.contains("secret_")
            || filename.contains("_secret")
        {
            return Some("secrets".to_string());
        }

        // 5. Sensitive keys/certificates (.pem, .key)
        if (filename.ends_with(".pem") || filename.ends_with(".key"))
            && !filename.ends_with(".keyboard")
        {
            return Some(filename.to_string());
        }

        None
    }

    /// Evaluates bash/shell command strings against destructive patterns.
    pub fn evaluate_bash_command(&self, args: &Value) -> Option<PolicyDecision> {
        let cmd = extract_command_str(args)?;
        let trimmed = cmd.trim();

        // 1. High-severity root/home wipe commands -> Deny
        if let Ok(re) = regex::Regex::new(
            r"(?i)\brm\s+(-[a-z]*r[a-z]*f|-[a-z]*f[a-z]*r|--recursive\s+--force|--force\s+--recursive)\s+(/\s*$|/\*|~\s*$|~/|\$HOME\b)",
        ) {
            if re.is_match(trimmed) {
                return Some(PolicyDecision::Deny(format!(
                    "Destructive root or home deletion prohibited: '{trimmed}'"
                )));
            }
        }

        // 2. Disk format command -> Deny
        if let Ok(re) = regex::Regex::new(r"(?i)\bformat(\.exe)?\s+([a-z]:|/fs:)") {
            if re.is_match(trimmed) {
                return Some(PolicyDecision::Deny(format!(
                    "Disk format command prohibited: '{trimmed}'"
                )));
            }
        }
        if let Ok(re) = regex::Regex::new(r"(?i)\bformat\s+[a-z]:") {
            if re.is_match(trimmed) {
                return Some(PolicyDecision::Deny(format!(
                    "Disk format command prohibited: '{trimmed}'"
                )));
            }
        }

        // 3. Raw filesystem creation (mkfs) -> Deny
        if let Ok(re) = regex::Regex::new(r"(?i)\bmkfs(\.[a-z0-9]+)?\b") {
            if re.is_match(trimmed) {
                return Some(PolicyDecision::Deny(format!(
                    "Filesystem creation (mkfs) prohibited: '{trimmed}'"
                )));
            }
        }

        // 4. Shell fork bomb -> Deny
        if trimmed.contains(":(){ :|:& };:") {
            return Some(PolicyDecision::Deny(
                "Shell fork bomb prohibited".to_string(),
            ));
        }

        // 5. Destructive directory deletion (rm -rf) -> Ask
        if let Ok(re) = regex::Regex::new(
            r"(?i)\brm\s+(-[a-z]*r[a-z]*f|-[a-z]*f[a-z]*r|--recursive\s+--force|--force\s+--recursive)\b",
        ) {
            if re.is_match(trimmed) {
                return Some(PolicyDecision::Ask(format!(
                    "Destructive directory deletion requires confirmation: '{trimmed}'"
                )));
            }
        }
        // rm -r -f or rm -f -r
        if let Ok(re1) = regex::Regex::new(r"(?i)\brm\s+-[a-z]*r[a-z]*\s+-[a-z]*f[a-z]*\b") {
            if re1.is_match(trimmed) {
                return Some(PolicyDecision::Ask(format!(
                    "Destructive directory deletion requires confirmation: '{trimmed}'"
                )));
            }
        }
        if let Ok(re2) = regex::Regex::new(r"(?i)\brm\s+-[a-z]*f[a-z]*\s+-[a-z]*r[a-z]*\b") {
            if re2.is_match(trimmed) {
                return Some(PolicyDecision::Ask(format!(
                    "Destructive directory deletion requires confirmation: '{trimmed}'"
                )));
            }
        }

        // 6. SQL Drop statements (drop table, drop database) -> Ask
        if let Ok(re) = regex::Regex::new(r"(?i)\bdrop\s+(table|database|schema)\b") {
            if re.is_match(trimmed) {
                return Some(PolicyDecision::Ask(format!(
                    "Destructive SQL operation requires confirmation: '{trimmed}'"
                )));
            }
        }

        None
    }
}

// ============================================================================
// Helpers
// ============================================================================

/// Extracts a shell command string from standard tool argument shapes.
pub fn extract_command_str(args: &Value) -> Option<String> {
    match args {
        Value::String(s) => Some(s.clone()),
        Value::Object(map) => {
            for key in ["command", "cmd", "script", "code"] {
                if let Some(Value::String(s)) = map.get(key) {
                    return Some(s.clone());
                }
            }
            if let Some(Value::Array(arr)) = map.get("args") {
                let parts: Vec<String> = arr
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect();
                if !parts.is_empty() {
                    return Some(parts.join(" "));
                }
            }
            None
        }
        Value::Array(arr) => {
            let parts: Vec<String> = arr
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect();
            if !parts.is_empty() {
                Some(parts.join(" "))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Extracts potential filesystem paths from tool arguments.
pub fn extract_all_paths(args: &Value) -> Vec<String> {
    let mut paths = Vec::new();
    match args {
        Value::String(s) => {
            paths.push(s.clone());
        }
        Value::Object(map) => {
            // First check common path keys
            for key in [
                "path",
                "file",
                "file_path",
                "filepath",
                "target",
                "dest",
                "destination",
                "source",
                "src",
                "filename",
                "dir",
                "directory",
            ] {
                if let Some(Value::String(s)) = map.get(key) {
                    paths.push(s.clone());
                }
            }
            // If no explicit path keys matched, collect all string values
            if paths.is_empty() {
                for v in map.values() {
                    if let Value::String(s) = v {
                        paths.push(s.clone());
                    }
                }
            }
        }
        Value::Array(arr) => {
            for v in arr {
                if let Value::String(s) = v {
                    paths.push(s.clone());
                }
            }
        }
        _ => {}
    }
    paths
}

fn matches_command_pattern(pattern: &str, args: &Value) -> bool {
    if let Some(cmd) = extract_command_str(args) {
        if cmd.contains(pattern) {
            return true;
        }
        if let Ok(re) = regex::Regex::new(pattern) {
            if re.is_match(&cmd) {
                return true;
            }
        }
    }

    let serialized = args.to_string();
    if serialized.contains(pattern) {
        return true;
    }
    if let Ok(re) = regex::Regex::new(pattern) {
        if re.is_match(&serialized) {
            return true;
        }
    }

    false
}

fn matches_path_pattern(pattern: &str, args: &Value) -> bool {
    let paths = extract_all_paths(args);
    for path in paths {
        if pattern == "*" || path.contains(pattern) {
            return true;
        }
        if pattern.starts_with('*') && path.ends_with(pattern.trim_start_matches('*')) {
            return true;
        }
        if let Ok(re) = regex::Regex::new(pattern) {
            if re.is_match(&path) {
                return true;
            }
        }
    }
    false
}
