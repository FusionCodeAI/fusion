use std::sync::Arc;
use std::time::Duration;

use futures::future::join_all;
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::provider::types::Message;
use crate::provider::LlmClient;
use crate::agent::consensus::{resolve_consensus, ConsensusResolution, ConsensusStrategy};

/// Assessed risk level from an advisor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

impl RiskLevel {
    /// Lowercase identifier used in JSON payloads and prompts.
    pub fn as_str(&self) -> &'static str {
        match self {
            RiskLevel::Low => "low",
            RiskLevel::Medium => "medium",
            RiskLevel::High => "high",
            RiskLevel::Critical => "critical",
        }
    }

    /// Parses a case-insensitive risk level string; unknown values degrade to Low.
    pub fn parse_risk_level(raw: &str) -> Self {
        match raw.trim().to_lowercase().as_str() {
            "critical" | "crit" | "severe" | "4" => RiskLevel::Critical,
            "high" | "3" => RiskLevel::High,
            "medium" | "med" | "moderate" | "2" => RiskLevel::Medium,
            _ => RiskLevel::Low,
        }
    }

    /// Returns true if the risk is Critical.
    pub fn is_critical(&self) -> bool {
        matches!(self, RiskLevel::Critical)
    }

    /// Returns true if the risk is High or Critical.
    pub fn is_high_or_critical(&self) -> bool {
        matches!(self, RiskLevel::High | RiskLevel::Critical)
    }
}

impl std::fmt::Display for RiskLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RiskLevel::Low => write!(f, "LOW"),
            RiskLevel::Medium => write!(f, "MEDIUM"),
            RiskLevel::High => write!(f, "HIGH"),
            RiskLevel::Critical => write!(f, "CRITICAL"),
        }
    }
}

/// A specialized advisor providing domain-specific critiques.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Advisor {
    pub name: String,
    pub focus: String,
    pub system_prompt: String,
}

impl Advisor {
    /// Creates a new advisor with a name, focus area, and system prompt.
    pub fn new(
        name: impl Into<String>,
        focus: impl Into<String>,
        system_prompt: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            focus: focus.into(),
            system_prompt: system_prompt.into(),
        }
    }

    /// Default Architecture Advisor.
    pub fn architecture() -> Self {
        Self::new(
            "ArchitectureAdvisor",
            "Software architecture, modularity, clean design, separation of concerns, and cross-platform compatibility",
            r#"You are Fusion's Architecture Advisor. Your mission is to review proposed plans, tool calls, and implementations for architectural integrity.
Look out for:
1. Modularity, clean layering, separation of concerns, and SOLID/DRY principles.
2. Cross-platform support (Linux, macOS, Windows, and Android/Termux: pure-Rust dependencies, no hardcoded /tmp or bash-specific paths).
3. Extensibility, low coupling, avoiding monolithic files or unnecessary abstractions.
4. Correct asynchronous lifecycle, concurrency safety, and deadlock prevention.

Respond in structured JSON format with this exact schema:
{
  "approved": true | false,
  "risk_level": "low" | "medium" | "high" | "critical",
  "critique": "Crisp 1-3 sentence explanation of architectural advice",
  "suggestions": ["Optional suggestion 1", "Optional suggestion 2"]
}"#,
        )
    }

    /// Default Security Advisor.
    pub fn security() -> Self {
        Self::new(
            "SecurityAdvisor",
            "Security, secrets, command safety, permission boundaries, and vulnerability prevention",
            r#"You are Fusion's Security Advisor. Your sole mission is to analyze user requests, proposed actions, and planned commands/tool calls for security vulnerabilities.
Look out for:
1. Destructive or accidental commands (e.g. rm -rf, mkfs, overwriting root files, deleting git history).
2. Secret leakage (exposing .env files, private keys, API keys, tokens, credentials).
3. Privilege escalation or unauthorized execution (arbitrary shell injection, curl | bash, piping untrusted data).
4. Unsafe file permissions or exposed network listeners.

Respond in structured JSON format with this exact schema:
{
  "approved": true | false,
  "risk_level": "low" | "medium" | "high" | "critical",
  "critique": "Crisp 1-3 sentence explanation of security risks or validation",
  "suggestions": ["Optional suggestion 1", "Optional suggestion 2"]
}"#,
        )
    }

    /// Default Code Review Advisor.
    pub fn code_review() -> Self {
        Self::new(
            "CodeReviewAdvisor",
            "Code quality, idiomatic patterns, performance, zero-allocation design, error handling, test coverage, and reliability",
            r#"You are Fusion's Code Review Advisor. Your mission is to evaluate code changes, technical implementation, and proposed tools for quality, correctness, and reliability.
Look out for:
1. Idiomatic Rust patterns, robust error handling with anyhow/thiserror instead of unwrap()/expect().
2. Performance efficiency: avoiding unnecessary allocations, copies, or redundant computations.
3. Proper documentation, testability, and edge cases (empty collections, invalid UTF-8, network timeouts).
4. Code clarity and simplicity: prefer boring, readable, maintainable solutions over complex wizardry.
5. Correct async/await lifecycle, deadlock prevention, and cancellation safety.

Respond in structured JSON format with this exact schema:
{
  "approved": true | false,
  "risk_level": "low" | "medium" | "high" | "critical",
  "critique": "Crisp 1-3 sentence explanation of code review feedback",
  "suggestions": ["Optional suggestion 1", "Optional suggestion 2"]
}"#,
        )
    }

    /// Alias for `code_review`.
    pub fn code_quality() -> Self {
        Self::code_review()
    }
}

/// Critique returned by an advisor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdvisorCritique {
    pub advisor: String,
    pub focus: String,
    pub approved: bool,
    pub risk_level: RiskLevel,
    pub critique: String,
    pub suggestions: Vec<String>,
}

impl AdvisorCritique {
    /// Returns true if the critique approved the action.
    pub fn is_approved(&self) -> bool {
        self.approved
    }

    /// Returns true if the critique identified a Critical risk.
    pub fn is_critical(&self) -> bool {
        self.risk_level.is_critical()
    }

    /// Returns true if the critique identified a High or Critical risk.
    pub fn is_high_or_critical(&self) -> bool {
        self.risk_level.is_high_or_critical()
    }
}

/// Internal JSON schema for parsing model responses.
#[derive(Debug, Deserialize)]
struct AdvisorResponseJson {
    approved: Option<bool>,
    risk_level: Option<String>,
    critique: Option<String>,
    suggestions: Option<Vec<String>>,
}

/// Registry of active advisors.
#[derive(Debug, Clone, Default)]
pub struct AdvisorRegistry {
    advisors: Vec<Advisor>,
}

impl AdvisorRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self {
            advisors: Vec::new(),
        }
    }

    /// Creates a registry pre-loaded with the default advisors (Architecture, Security, CodeReview).
    pub fn default_advisors() -> Self {
        let mut reg = Self::new();
        reg.register(Advisor::architecture());
        reg.register(Advisor::security());
        reg.register(Advisor::code_review());
        reg
    }

    /// Registers a new advisor.
    pub fn register(&mut self, advisor: Advisor) {
        self.advisors.push(advisor);
    }

    /// Returns a slice of all registered advisors.
    pub fn all(&self) -> &[Advisor] {
        &self.advisors
    }

    /// Looks up an advisor by name.
    pub fn get(&self, name: &str) -> Option<&Advisor> {
        self.advisors.iter().find(|a| a.name.eq_ignore_ascii_case(name))
    }

    /// Returns true if no advisors are registered.
    pub fn is_empty(&self) -> bool {
        self.advisors.is_empty()
    }

    /// Returns the count of registered advisors.
    pub fn len(&self) -> usize {
        self.advisors.len()
    }

    /// Returns names of all advisors.
    pub fn names(&self) -> Vec<&str> {
        self.advisors.iter().map(|a| a.name.as_str()).collect()
    }
}

impl From<&AdvisorRegistry> for Vec<Advisor> {
    fn from(reg: &AdvisorRegistry) -> Self {
        reg.all().to_vec()
    }
}

/// Engine that coordinates advisor management and parallel advisory query fan-out.
#[derive(Clone)]
pub struct AdvisorEngine {
    client: LlmClient,
    config: Config,
    advisors: Vec<Advisor>,
}

impl AdvisorEngine {
    /// Creates a new AdvisorEngine with default advisors (Architecture, Security, CodeReview).
    pub fn new(client: LlmClient, config: Config) -> Self {
        Self {
            client,
            config,
            advisors: Self::default_advisors(),
        }
    }

    /// Creates an AdvisorEngine with a custom list of advisors.
    pub fn with_advisors(client: LlmClient, config: Config, advisors: Vec<Advisor>) -> Self {
        Self {
            client,
            config,
            advisors,
        }
    }

    /// Creates an AdvisorEngine from an AdvisorRegistry.
    pub fn from_registry(client: LlmClient, config: Config, registry: &AdvisorRegistry) -> Self {
        Self {
            client,
            config,
            advisors: registry.all().to_vec(),
        }
    }

    /// Returns the default set of advisors: Architecture, Security, and CodeReview.
    pub fn default_advisors() -> Vec<Advisor> {
        vec![
            Advisor::architecture(),
            Advisor::security(),
            Advisor::code_review(),
        ]
    }

    /// Registers an additional advisor.
    pub fn add_advisor(&mut self, advisor: Advisor) {
        self.advisors.push(advisor);
    }

    /// Alias for `add_advisor`.
    pub fn register(&mut self, advisor: Advisor) {
        self.add_advisor(advisor);
    }

    /// Returns a slice of active advisors.
    pub fn advisors(&self) -> &[Advisor] {
        &self.advisors
    }

    /// Returns a mutable reference to the active advisors list.
    pub fn advisors_mut(&mut self) -> &mut Vec<Advisor> {
        &mut self.advisors
    }

    /// Returns a reference to the LLM client.
    pub fn client(&self) -> &LlmClient {
        &self.client
    }

    /// Returns a reference to the configuration.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Updates the configuration.
    pub fn set_config(&mut self, config: Config) {
        self.config = config;
    }

    /// Returns true if advisor critiques are enabled and there are active advisors.
    pub fn is_enabled(&self) -> bool {
        self.config.advisors_enabled && !self.advisors.is_empty()
    }

    /// Fans out consultation queries in parallel to all active advisors to critique a proposed plan.
    pub async fn critique_plan(
        &self,
        user_request: &str,
        proposed_plan: &str,
    ) -> Vec<AdvisorCritique> {
        if !self.is_enabled() {
            return Vec::new();
        }

        let action_description = if proposed_plan.trim().is_empty() {
            String::new()
        } else {
            format!("PROPOSED PLAN / IMPLEMENTATION STRATEGY:\n{}", proposed_plan)
        };

        self.consult(user_request, &action_description).await
    }

    /// Fans out consultation queries in parallel to all active advisors to critique a tool call before execution.
    pub async fn critique_tool_call(
        &self,
        user_request: &str,
        tool_name: &str,
        tool_args: &serde_json::Value,
    ) -> Vec<AdvisorCritique> {
        if !self.is_enabled() {
            return Vec::new();
        }

        let args_formatted = serde_json::to_string_pretty(tool_args)
            .unwrap_or_else(|_| tool_args.to_string());

        let action_description = format!(
            "PROPOSED TOOL CALL (PRE-EXECUTION CHECK):\nTool Name: {}\nArguments:\n{}",
            tool_name, args_formatted
        );

        self.consult(user_request, &action_description).await
    }

    /// General consultation method fanning out parallel advisory queries.
    pub async fn consult(
        &self,
        user_request: &str,
        proposed_action: &str,
    ) -> Vec<AdvisorCritique> {
        consult_advisors(
            &self.advisors,
            user_request,
            proposed_action,
            &self.client,
            &self.config,
        )
        .await
    }

    /// Checks if all critiques are approved.
    pub fn is_all_approved(critiques: &[AdvisorCritique]) -> bool {
        critiques.iter().all(|c| c.approved)
    }

    /// Checks if any critique identified a Critical risk.
    pub fn has_critical_risk(critiques: &[AdvisorCritique]) -> bool {
        critiques.iter().any(|c| c.risk_level == RiskLevel::Critical)
    }

    /// Checks if any critique identified a High or Critical risk.
    pub fn has_high_or_critical_risk(critiques: &[AdvisorCritique]) -> bool {
        critiques.iter().any(|c| c.risk_level >= RiskLevel::High)
    }

    /// Computes the highest risk level among all critiques.
    pub fn highest_risk(critiques: &[AdvisorCritique]) -> RiskLevel {
        critiques
            .iter()
            .map(|c| c.risk_level)
            .max()
            .unwrap_or(RiskLevel::Low)
    }

    /// Formats a list of critiques into a formatted summary.
    pub fn format_critiques(critiques: &[AdvisorCritique]) -> String {
        format_critiques_for_system_prompt(critiques)
    }

    /// Evaluates critiques according to the given consensus strategy.
    pub fn resolve_consensus(
        critiques: &[AdvisorCritique],
        strategy: ConsensusStrategy,
    ) -> ConsensusResolution {
        resolve_consensus(critiques, strategy)
    }

    /// Queries all advisors in parallel and immediately resolves the consensus.
    pub async fn consult_with_consensus(
        &self,
        user_request: &str,
        proposed_action: &str,
        strategy: ConsensusStrategy,
    ) -> (Vec<AdvisorCritique>, ConsensusResolution) {
        let critiques = self.consult(user_request, proposed_action).await;
        let resolution = resolve_consensus(&critiques, strategy);
        (critiques, resolution)
    }

    /// Reviews a tool call in parallel and immediately resolves the consensus.
    pub async fn critique_tool_call_with_consensus(
        &self,
        user_request: &str,
        tool_name: &str,
        tool_args: &serde_json::Value,
        strategy: ConsensusStrategy,
    ) -> (Vec<AdvisorCritique>, ConsensusResolution) {
        let critiques = self.critique_tool_call(user_request, tool_name, tool_args).await;
        let resolution = resolve_consensus(&critiques, strategy);
        (critiques, resolution)
    }
}

/// Fans out consultation queries to all provided advisors in parallel.
/// Returns a list of critiques collected from each advisor.
pub async fn consult_advisors(
    advisors: &[Advisor],
    user_request: &str,
    proposed_action: &str,
    client: &LlmClient,
    config: &Config,
) -> Vec<AdvisorCritique> {
    if advisors.is_empty() || !config.advisors_enabled {
        return Vec::new();
    }

    // Clone config and use advisor_model if configured
    let mut advisor_config = config.clone();
    if let Some(model) = &config.advisor_model {
        advisor_config.default_model = model.clone();
    }

    let advisor_config = Arc::new(advisor_config);
    let user_request = Arc::new(user_request.to_string());
    let proposed_action = Arc::new(proposed_action.to_string());
    let client_arc = Arc::new(client.clone());

    let mut futures = Vec::with_capacity(advisors.len());

    for advisor in advisors {
        let advisor = advisor.clone();
        let client = Arc::clone(&client_arc);
        let config = Arc::clone(&advisor_config);
        let req = Arc::clone(&user_request);
        let action = Arc::clone(&proposed_action);

        futures.push(tokio::spawn(async move {
            consult_single_advisor(&advisor, &req, &action, &client, &config).await
        }));
    }

    let results = join_all(futures).await;
    let mut critiques = Vec::with_capacity(results.len());

    for res in results {
        match res {
            Ok(critique) => critiques.push(critique),
            Err(e) => {
                tracing::warn!("Advisor task join error: {}", e);
            }
        }
    }

    critiques
}

/// Queries a single advisor with the prompt and parses its structured critique.
async fn consult_single_advisor(
    advisor: &Advisor,
    user_request: &str,
    proposed_action: &str,
    client: &LlmClient,
    config: &Config,
) -> AdvisorCritique {
    let mut messages = Vec::new();

    // System prompt tailored for this advisor
    messages.push(Message::system(&advisor.system_prompt));

    // User prompt providing context
    let prompt_content = if proposed_action.trim().is_empty() {
        format!(
            "User Request:\n{}\n\nPlease evaluate this user request from your perspective as the {} (focus: {}). Identify any risks, constraints, or best practices to keep in mind.",
            user_request, advisor.name, advisor.focus
        )
    } else {
        format!(
            "User Request:\n{}\n\nProposed Action / Plan:\n{}\n\nPlease evaluate this request and proposed action from your perspective as the {} (focus: {}).",
            user_request, proposed_action, advisor.name, advisor.focus
        )
    };

    messages.push(Message::user(prompt_content));

    // Perform completion with a 30-second timeout to prevent stalling
    let completion_result = tokio::time::timeout(
        Duration::from_secs(30),
        client.complete(config, &messages, &[]),
    )
    .await;

    match completion_result {
        Ok(Ok((content, _reasoning, _tool_calls))) => {
            parse_advisor_response(advisor, &content)
        }
        Ok(Err(e)) => {
            tracing::warn!("Advisor '{}' request failed: {}", advisor.name, e);
            AdvisorCritique {
                advisor: advisor.name.clone(),
                focus: advisor.focus.clone(),
                approved: true,
                risk_level: RiskLevel::Low,
                critique: format!("Advisor consultation unavailable: {}", e),
                suggestions: Vec::new(),
            }
        }
        Err(_) => {
            tracing::warn!("Advisor '{}' request timed out", advisor.name);
            AdvisorCritique {
                advisor: advisor.name.clone(),
                focus: advisor.focus.clone(),
                approved: true,
                risk_level: RiskLevel::Low,
                critique: "Advisor consultation timed out".to_string(),
                suggestions: Vec::new(),
            }
        }
    }
}

/// Parses the LLM's response into an `AdvisorCritique`.
fn parse_advisor_response(advisor: &Advisor, raw_content: &str) -> AdvisorCritique {
    let trimmed = raw_content.trim();

    // Extract JSON from a fenced code block, an inline JSON object, or the raw text.
    let json_str = if let Some(start) = trimmed.find("```json") {
        let after_start = &trimmed[start + 7..];
        match after_start.find("```") {
            Some(end) => after_start[..end].trim(),
            None => after_start.trim(),
        }
    } else if let Some(start) = trimmed.find("```") {
        let after_start = &trimmed[start + 3..];
        match after_start.find("```") {
            Some(end) => after_start[..end].trim(),
            None => after_start.trim(),
        }
    } else if let Some(start) = trimmed.find('{') {
        if let Some(end) = trimmed.rfind('}') {
            &trimmed[start..=end]
        } else {
            trimmed
        }
    } else {
        trimmed
    };

    if let Ok(parsed) = serde_json::from_str::<AdvisorResponseJson>(json_str) {
        let risk = parsed
            .risk_level
            .as_deref()
            .map(RiskLevel::parse_risk_level)
            .unwrap_or(RiskLevel::Low);

        return AdvisorCritique {
            advisor: advisor.name.clone(),
            focus: advisor.focus.clone(),
            approved: parsed.approved.unwrap_or(risk <= RiskLevel::Medium),
            risk_level: risk,
            critique: parsed.critique.unwrap_or_else(|| trimmed.to_string()),
            suggestions: parsed.suggestions.unwrap_or_default(),
        };
    }

    // Fallback: parse plain text response
    let lower = trimmed.to_lowercase();
    let approved = !lower.contains("disapproved")
        && !lower.contains("critical risk")
        && !lower.contains("reject")
        && !lower.contains("dangerous action blocked");

    let risk_level = if lower.contains("critical") {
        RiskLevel::Critical
    } else if lower.contains("high risk") || lower.contains("danger") {
        RiskLevel::High
    } else if lower.contains("medium risk")
        || lower.contains("warning")
        || lower.contains("caution")
    {
        RiskLevel::Medium
    } else {
        RiskLevel::Low
    };

    AdvisorCritique {
        advisor: advisor.name.clone(),
        focus: advisor.focus.clone(),
        approved,
        risk_level,
        critique: trimmed.to_string(),
        suggestions: Vec::new(),
    }
}

/// Formats a list of advisor critiques into markdown suitable for injection into system messages or context.
pub fn format_critiques_for_system_prompt(critiques: &[AdvisorCritique]) -> String {
    if critiques.is_empty() {
        return String::new();
    }

    let mut out = String::from("\n### Advisor Critiques & Safety Notes:\n");
    for c in critiques {
        let status_emoji = if c.approved {
            "[APPROVED]"
        } else {
            "[FLAGGED/WARNING]"
        };
        out.push_str(&format!(
            "- **{}** ({}, Risk: {}): {}\n  {}\n",
            c.advisor, c.focus, c.risk_level, status_emoji, c.critique
        ));
        for s in &c.suggestions {
            out.push_str(&format!("  * Suggestion: {}\n", s));
        }
    }
    out
}

/// Formats a concise summary of advisor critiques for UI status display.
pub fn format_critiques_summary(critiques: &[AdvisorCritique]) -> String {
    if critiques.is_empty() {
        return String::from("No advisor critiques.");
    }

    let mut parts = Vec::new();
    for c in critiques {
        let status = if c.approved { "✓" } else { "⚠" };
        parts.push(format!("{}: {} ({})", c.advisor, status, c.risk_level));
    }
    parts.join(" | ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_risk_level_ordering_and_display() {
        assert!(RiskLevel::Low < RiskLevel::Medium);
        assert!(RiskLevel::Medium < RiskLevel::High);
        assert!(RiskLevel::High < RiskLevel::Critical);

        assert_eq!(RiskLevel::Low.to_string(), "LOW");
        assert_eq!(RiskLevel::Critical.to_string(), "CRITICAL");
        assert!(RiskLevel::Critical.is_critical());
        assert!(RiskLevel::Critical.is_high_or_critical());
        assert!(RiskLevel::High.is_high_or_critical());
        assert!(!RiskLevel::Medium.is_high_or_critical());
    }

    #[test]
    fn test_default_advisors_creation() {
        let arch = Advisor::architecture();
        assert_eq!(arch.name, "ArchitectureAdvisor");
        assert!(arch.focus.contains("architecture"));

        let sec = Advisor::security();
        assert_eq!(sec.name, "SecurityAdvisor");
        assert!(sec.focus.contains("Security"));

        let review = Advisor::code_review();
        assert_eq!(review.name, "CodeReviewAdvisor");
        assert!(review.focus.contains("Code quality"));

        let quality = Advisor::code_quality();
        assert_eq!(quality.name, "CodeReviewAdvisor");
    }

    #[test]
    fn test_advisor_registry() {
        let mut reg = AdvisorRegistry::new();
        assert!(reg.is_empty());
        assert_eq!(reg.len(), 0);

        reg.register(Advisor::security());
        assert_eq!(reg.len(), 1);
        assert!(!reg.is_empty());
        assert!(reg.get("securityadvisor").is_some());
        assert!(reg.get("nonexistent").is_none());

        let default_reg = AdvisorRegistry::default_advisors();
        assert_eq!(default_reg.len(), 3);
        assert_eq!(
            default_reg.names(),
            vec!["ArchitectureAdvisor", "SecurityAdvisor", "CodeReviewAdvisor"]
        );
    }

    #[test]
    fn test_advisor_engine_initialization() {
        let client = LlmClient::new();
        let config = Config::default();

        let engine = AdvisorEngine::new(client.clone(), config.clone());
        assert_eq!(engine.advisors().len(), 3);
        assert!(engine.is_enabled());

        let mut custom_engine = AdvisorEngine::with_advisors(client, config, Vec::new());
        assert_eq!(custom_engine.advisors().len(), 0);
        assert!(!custom_engine.is_enabled());

        custom_engine.add_advisor(Advisor::security());
        assert_eq!(custom_engine.advisors().len(), 1);
        assert!(custom_engine.is_enabled());
    }

    #[test]
    fn test_parse_advisor_response_json() {
        let advisor = Advisor::security();
        let raw_json = r#"{
            "approved": false,
            "risk_level": "critical",
            "critique": "Command deletes root directory.",
            "suggestions": ["Do not run rm -rf /"]
        }"#;

        let critique = parse_advisor_response(&advisor, raw_json);
        assert_eq!(critique.advisor, "SecurityAdvisor");
        assert!(!critique.approved);
        assert_eq!(critique.risk_level, RiskLevel::Critical);
        assert_eq!(critique.critique, "Command deletes root directory.");
        assert_eq!(critique.suggestions, vec!["Do not run rm -rf /"]);
    }

    #[test]
    fn test_parse_advisor_response_markdown_code_fence() {
        let advisor = Advisor::architecture();
        let raw_md = "Here is my evaluation:\n```json\n{\n  \"approved\": true,\n  \"risk_level\": \"low\",\n  \"critique\": \"Clean modular layout.\",\n  \"suggestions\": []\n}\n```\nHope this helps!";

        let critique = parse_advisor_response(&advisor, raw_md);
        assert_eq!(critique.advisor, "ArchitectureAdvisor");
        assert!(critique.approved);
        assert_eq!(critique.risk_level, RiskLevel::Low);
        assert_eq!(critique.critique, "Clean modular layout.");
    }

    #[test]
    fn test_parse_advisor_response_fallback_text() {
        let advisor = Advisor::code_review();
        let text = "This plan looks fine and approved with no issues.";

        let critique = parse_advisor_response(&advisor, text);
        assert_eq!(critique.advisor, "CodeReviewAdvisor");
        assert!(critique.approved);
        assert_eq!(critique.risk_level, RiskLevel::Low);

        let dangerous_text = "This action has CRITICAL risk and is rejected.";
        let bad_critique = parse_advisor_response(&advisor, dangerous_text);
        assert!(!bad_critique.approved);
        assert_eq!(bad_critique.risk_level, RiskLevel::Critical);
    }

    #[test]
    fn test_critique_evaluation_helpers() {
        let critiques = vec![
            AdvisorCritique {
                advisor: "ArchitectureAdvisor".to_string(),
                focus: "Architecture".to_string(),
                approved: true,
                risk_level: RiskLevel::Low,
                critique: "Looks good.".to_string(),
                suggestions: vec![],
            },
            AdvisorCritique {
                advisor: "SecurityAdvisor".to_string(),
                focus: "Security".to_string(),
                approved: false,
                risk_level: RiskLevel::Critical,
                critique: "Dangerous command.".to_string(),
                suggestions: vec!["Refactor command".to_string()],
            },
        ];

        assert!(!AdvisorEngine::is_all_approved(&critiques));
        assert!(AdvisorEngine::has_critical_risk(&critiques));
        assert!(AdvisorEngine::has_high_or_critical_risk(&critiques));
        assert_eq!(AdvisorEngine::highest_risk(&critiques), RiskLevel::Critical);

        let formatted = format_critiques_for_system_prompt(&critiques);
        assert!(formatted.contains("ArchitectureAdvisor"));
        assert!(formatted.contains("SecurityAdvisor"));
        assert!(formatted.contains("[FLAGGED/WARNING]"));

        let summary = format_critiques_summary(&critiques);
        assert!(summary.contains("ArchitectureAdvisor: ✓ (LOW)"));
        assert!(summary.contains("SecurityAdvisor: ⚠ (CRITICAL)"));
    }

    #[test]
    fn test_risk_level_as_str_and_roundtrip() {
        assert_eq!(RiskLevel::Low.as_str(), "low");
        assert_eq!(RiskLevel::Medium.as_str(), "medium");
        assert_eq!(RiskLevel::High.as_str(), "high");
        assert_eq!(RiskLevel::Critical.as_str(), "critical");

        for level in [
            RiskLevel::Low,
            RiskLevel::Medium,
            RiskLevel::High,
            RiskLevel::Critical,
        ] {
            assert_eq!(RiskLevel::parse_risk_level(level.as_str()), level);
        }
    }

    #[test]
    fn test_parse_risk_level_case_and_variants() {
        assert_eq!(RiskLevel::parse_risk_level("CRITICAL"), RiskLevel::Critical);
        assert_eq!(RiskLevel::parse_risk_level("  High  "), RiskLevel::High);
        assert_eq!(RiskLevel::parse_risk_level("moderate"), RiskLevel::Medium);
        assert_eq!(RiskLevel::parse_risk_level("med"), RiskLevel::Medium);
        assert_eq!(RiskLevel::parse_risk_level("severe"), RiskLevel::Critical);
        assert_eq!(RiskLevel::parse_risk_level("4"), RiskLevel::Critical);
        assert_eq!(RiskLevel::parse_risk_level("2"), RiskLevel::Medium);

        // Unknown values degrade to Low rather than failing.
        assert_eq!(RiskLevel::parse_risk_level(""), RiskLevel::Low);
        assert_eq!(RiskLevel::parse_risk_level("bananas"), RiskLevel::Low);
    }

    #[test]
    fn test_parse_advisor_response_inline_json_object() {
        // JSON embedded in surrounding prose without code fences must be extracted.
        let advisor = Advisor::security();
        let raw = "Sure! Here is my assessment: {\"approved\": false, \"risk_level\": \"HIGH\", \"critique\": \"Pipes untrusted input into a shell.\", \"suggestions\": [\"Validate input first\"]} Thanks!";

        let critique = parse_advisor_response(&advisor, raw);
        assert_eq!(critique.advisor, "SecurityAdvisor");
        assert!(!critique.approved);
        assert_eq!(critique.risk_level, RiskLevel::High);
        assert_eq!(critique.critique, "Pipes untrusted input into a shell.");
        assert_eq!(critique.suggestions, vec!["Validate input first"]);
    }

    #[test]
    fn test_parse_advisor_response_missing_fields_defaults() {
        let advisor = Advisor::architecture();

        // Missing risk_level defaults to Low; missing approved defaults true for Low risk.
        let partial = r#"{"critique": "Reasonable plan."}"#;
        let critique = parse_advisor_response(&advisor, partial);
        assert!(critique.approved);
        assert_eq!(critique.risk_level, RiskLevel::Low);
        assert_eq!(critique.critique, "Reasonable plan.");
        assert!(critique.suggestions.is_empty());

        // Missing approved defaults false when the assessed risk is High.
        let partial_high = r#"{"risk_level": "critical", "critique": "Monolithic design risk."}"#;
        let high_critique = parse_advisor_response(&advisor, partial_high);
        assert!(!high_critique.approved);
        assert_eq!(high_critique.risk_level, RiskLevel::Critical);
    }

    #[test]
    fn test_parse_advisor_response_risk_string_variants() {
        let advisor = Advisor::code_review();

        for (raw, expected) in [
            (r#"{"approved": true, "risk_level": "low"}"#, RiskLevel::Low),
            (r#"{"approved": true, "risk_level": "Medium"}"#, RiskLevel::Medium),
            (r#"{"approved": false, "risk_level": "high"}"#, RiskLevel::High),
            (r#"{"approved": false, "risk_level": "severe"}"#, RiskLevel::Critical),
            (r#"{"approved": true, "risk_level": "mystery"}"#, RiskLevel::Low),
        ] {
            let critique = parse_advisor_response(&advisor, raw);
            assert_eq!(critique.risk_level, expected, "raw: {raw}");
        }
    }

    #[test]
    fn test_parse_advisor_response_non_json_flagged_text() {
        // Non-JSON responses with caution words fall back to text heuristics.
        let advisor = Advisor::security();

        let warn = parse_advisor_response(&advisor, "Proceed with caution: the command overwrites a config file.");
        assert!(warn.approved);
        assert_eq!(warn.risk_level, RiskLevel::Medium);

        let danger = parse_advisor_response(&advisor, "Danger: this deletes git history.");
        assert_eq!(danger.risk_level, RiskLevel::High);

        let blocked = parse_advisor_response(&advisor, "Disapproved: secret leakage risk in proposed action.");
        assert!(!blocked.approved);
    }

    #[test]
    fn test_parse_advisor_response_preserves_focus() {
        let advisor = Advisor::architecture();
        let critique = parse_advisor_response(&advisor, r#"{"approved": true, "risk_level": "low", "critique": "OK"}"#);
        assert_eq!(critique.focus, advisor.focus);
    }

    #[test]
    fn test_critique_evaluation_helpers_empty_and_low() {
        // Empty set: all-approved vacuously true, highest risk defaults to Low.
        assert!(AdvisorEngine::is_all_approved(&[]));
        assert!(!AdvisorEngine::has_critical_risk(&[]));
        assert!(!AdvisorEngine::has_high_or_critical_risk(&[]));
        assert_eq!(AdvisorEngine::highest_risk(&[]), RiskLevel::Low);

        // Low-risk approvals only.
        let low = vec![AdvisorCritique {
            advisor: "ArchitectureAdvisor".to_string(),
            focus: "Architecture".to_string(),
            approved: true,
            risk_level: RiskLevel::Low,
            critique: "Fine.".to_string(),
            suggestions: vec![],
        }];
        assert!(AdvisorEngine::is_all_approved(&low));
        assert!(!AdvisorEngine::has_high_or_critical_risk(&low));
        assert_eq!(AdvisorEngine::highest_risk(&low), RiskLevel::Low);
    }

    #[test]
    fn test_format_critiques_helpers_empty() {
        assert_eq!(format_critiques_for_system_prompt(&[]), String::new());
        assert_eq!(format_critiques_summary(&[]), "No advisor critiques.");
    }

    #[test]
    fn test_format_critiques_for_system_prompt_renders_suggestions() {
        let critiques = vec![AdvisorCritique {
            advisor: "SecurityAdvisor".to_string(),
            focus: "Security".to_string(),
            approved: false,
            risk_level: RiskLevel::High,
            critique: "Secrets in diff.".to_string(),
            suggestions: vec!["Strip secrets before commit".to_string()],
        }];

        let formatted = format_critiques_for_system_prompt(&critiques);
        assert!(formatted.contains("### Advisor Critiques & Safety Notes:"));
        assert!(formatted.contains("Risk: HIGH"));
        assert!(formatted.contains("Strip secrets before commit"));

        let summary = format_critiques_summary(&critiques);
        assert!(summary.contains("SecurityAdvisor: ⚠ (HIGH)"));
    }

    #[test]
    fn test_advisor_critique_predicates() {
        let critique = AdvisorCritique {
            advisor: "CodeReviewAdvisor".to_string(),
            focus: "Code quality".to_string(),
            approved: true,
            risk_level: RiskLevel::Medium,
            critique: "Minor concerns.".to_string(),
            suggestions: vec![],
        };

        assert!(critique.is_approved());
        assert!(!critique.is_critical());
        assert!(!critique.is_high_or_critical());

        let critical = AdvisorCritique {
            risk_level: RiskLevel::Critical,
            ..critique.clone()
        };
        assert!(critical.is_critical());
        assert!(critical.is_high_or_critical());
    }

    #[test]
    fn test_advisor_equality_and_registry_lookup() {
        let a = Advisor::security();
        let b = Advisor::security();
        let other = Advisor::code_review();

        assert_eq!(a, b);
        assert_ne!(a, other);

        let mut reg = AdvisorRegistry::default_advisors();
        assert!(reg.get("SecurityAdvisor").is_some());
        assert!(reg.get("SECURITYADVISOR").is_some());
        assert!(reg.get("Securityadvisor").is_some());

        let copied: Vec<Advisor> = (&reg).into();
        assert_eq!(copied.len(), 3);
        assert_eq!(copied[1], Advisor::security());
    }

    #[test]
    fn test_registry_duplicate_names_last_wins_lookup() {
        let mut reg = AdvisorRegistry::new();
        reg.register(Advisor::new("CustomAdvisor", "focus-a", "prompt-a"));
        reg.register(Advisor::new("CustomAdvisor", "focus-b", "prompt-b"));
        assert_eq!(reg.len(), 2);

        let found = reg.get("customadvisor").expect("duplicate name must be found");
        assert_eq!(found.focus, "focus-a");
    }

    #[test]
    fn test_consensus_resolution_via_engine() {
        let client = LlmClient::new();
        let config = Config::default();
        let engine = AdvisorEngine::with_advisors(client, config, Vec::new());

        let critiques = vec![AdvisorCritique {
            advisor: "ArchitectureAdvisor".to_string(),
            focus: "Architecture".to_string(),
            approved: true,
            risk_level: RiskLevel::Low,
            critique: "Clean.".to_string(),
            suggestions: vec![],
        }];

        let resolution = AdvisorEngine::resolve_consensus(&critiques, ConsensusStrategy::Unanimous);
        assert!(resolution.is_approved());
        assert_eq!(resolution.total_advisors, 1);
        assert_eq!(resolution.approved_count, 1);

        let dissent = vec![AdvisorCritique {
            advisor: "SecurityAdvisor".to_string(),
            focus: "Security".to_string(),
            approved: false,
            risk_level: RiskLevel::Critical,
            critique: "Veto.".to_string(),
            suggestions: vec![],
        }];
        let rejected = AdvisorEngine::resolve_consensus(&dissent, ConsensusStrategy::SecurityVeto);
        assert!(!rejected.is_approved());
    }

    #[tokio::test]
    async fn test_consult_with_consensus_structure() {
        let client = LlmClient::new();
        let mut config = Config::default();
        config.advisors_enabled = false; // Avoid real network calls in the test.

        let engine = AdvisorEngine::new(client, config);
        let (critiques, resolution) = engine
            .consult_with_consensus("Refactor module", "Split file in two", ConsensusStrategy::Unanimous)
            .await;
        assert!(critiques.is_empty());
        assert_eq!(resolution.total_advisors, 0);
        assert!(resolution.is_approved());
    }
}
