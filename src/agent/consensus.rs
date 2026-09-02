//! Multi-agent voting and consensus algorithms for parallel advisor reviews.
//!
//! When multiple domain-specific advisors (e.g. Security, Architecture, Code Review)
//! evaluate a user request, plan, or proposed tool execution in parallel, their
//! critiques must be synthesized into a collective consensus decision.
//!
//! This module provides flexible, deterministic consensus strategies:
//! - **Majority**: Strict or fractional majority voting across all participating advisors.
//! - **Unanimous**: Requires 100% approval from all advisors with zero dissenting votes.
//! - **RiskWeighted**: Weights votes dynamically based on assessed risk levels (Low, Medium,
//!   High, Critical), penalizing high-risk critiques and enforcing critical-risk vetoes.
//! - **Supermajority**: Requires a configurable fraction (e.g. 2/3 or 3/4) of approvals.
//! - **SecurityVeto**: Security domain critiques or critical risks exercise absolute veto power,
//!   while non-security advisors decide by majority vote.
//! - **CustomWeighted**: Custom per-advisor weights for tailored review topologies.

use std::collections::{HashMap, HashSet};
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::agent::advisor::{AdvisorCritique, RiskLevel};

/// Available voting and consensus strategies for aggregating advisor critiques.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "config", rename_all = "snake_case")]
pub enum ConsensusStrategy {
    /// Strict majority voting (> 50% of advisor approvals required).
    Majority,
    /// Unanimous consensus (100% of advisors must approve, 0 disapprovals allowed).
    Unanimous,
    /// Risk-weighted voting where higher risk levels carry exponential negative weight.
    RiskWeighted,
    /// Supermajority voting requiring at least the specified threshold fraction (e.g. 0.67 for 2/3).
    Supermajority(f64),
    /// Security-first veto: Security advisor and critical risks have absolute veto power;
    /// remaining advisors vote by majority.
    SecurityVeto,
    /// Custom advisor weighting by advisor name.
    CustomWeighted {
        weights: HashMap<String, f64>,
        default_weight: f64,
    },
}

impl Default for ConsensusStrategy {
    fn default() -> Self {
        Self::RiskWeighted
    }
}

impl fmt::Display for ConsensusStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Majority => write!(f, "Majority (>50%)"),
            Self::Unanimous => write!(f, "Unanimous (100%)"),
            Self::RiskWeighted => write!(f, "Risk-Weighted"),
            Self::Supermajority(thresh) => {
                write!(f, "Supermajority ({:.1}%)", thresh * 100.0)
            }
            Self::SecurityVeto => write!(f, "Security Veto"),
            Self::CustomWeighted { .. } => write!(f, "Custom-Weighted"),
        }
    }
}

/// Detailed evaluation of an individual advisor's vote within a consensus resolution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdvisorVote {
    /// Name of the advisor (e.g. "SecurityAdvisor", "ArchitectureAdvisor").
    pub advisor: String,
    /// Domain focus area of the advisor.
    pub focus: String,
    /// Whether the advisor approved the proposed action.
    pub approved: bool,
    /// Assessed risk level.
    pub risk_level: RiskLevel,
    /// Effective numerical weight assigned to this vote during calculation.
    pub effective_weight: f64,
    /// Normalized score contribution from this advisor (-weight to +weight).
    pub score_contribution: f64,
    /// Whether this advisor triggered a hard veto.
    pub vetoed: bool,
    /// Detailed critique from the advisor.
    pub critique: String,
    /// Specific suggestions proposed by the advisor.
    pub suggestions: Vec<String>,
}

/// The final outcome and detailed metadata of a consensus resolution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConsensusResolution {
    /// Final consensus determination (true if approved, false if rejected or vetoed).
    pub approved: bool,
    /// Consensus strategy used to evaluate the critiques.
    pub strategy: ConsensusStrategy,
    /// Overall confidence score of the consensus (0.0 to 1.0).
    pub confidence: f64,
    /// Total number of advisors that participated in the review.
    pub total_advisors: usize,
    /// Number of advisors that approved the action.
    pub approved_count: usize,
    /// Number of advisors that disapproved or flagged concerns.
    pub rejected_count: usize,
    /// Highest risk level assessed across all advisor critiques.
    pub highest_risk: RiskLevel,
    /// Aggregate risk score on a scale from 0.0 (no risk) to 10.0 (critical danger).
    ///
    /// Blends the mean advisor risk with the peak assessed risk
    /// (`0.6 * mean + 0.4 * peak`) so that a single severe dissent cannot be
    /// diluted by a large pool of low-risk approvals.
    pub aggregate_risk_score: f64,
    /// Concise human-readable summary of the consensus determination.
    pub summary: String,
    /// Specific reasons if any advisor or policy triggered a hard veto.
    pub veto_reasons: Vec<String>,
    /// List of dissenting critiques (advisors who voted against approval).
    pub dissenting_critiques: Vec<AdvisorCritique>,
    /// Aggregated and deduplicated suggestions across all participating advisors.
    pub recommendations: Vec<String>,
    /// Individual vote breakdown per advisor.
    pub votes: Vec<AdvisorVote>,
}

impl ConsensusResolution {
    /// Returns true if the proposed action passed consensus.
    pub fn is_approved(&self) -> bool {
        self.approved
    }

    /// Returns true if any advisor vetoed or if a critical risk was raised.
    pub fn has_veto(&self) -> bool {
        !self.veto_reasons.is_empty()
    }

    /// Returns true if the highest risk level is Critical.
    pub fn is_critical(&self) -> bool {
        self.highest_risk == RiskLevel::Critical
    }

    /// Returns true if the highest risk level is High or Critical.
    pub fn is_high_or_critical(&self) -> bool {
        self.highest_risk >= RiskLevel::High
    }

    /// Generates a single-line terminal/status badge for this resolution.
    pub fn status_badge(&self) -> String {
        let status = if self.approved {
            "✓ APPROVED"
        } else {
            "✗ REJECTED"
        };
        format!(
            "[{status}] (strategy: {}, {}/{} approved, risk: {}, confidence: {:.0}%)",
            self.strategy,
            self.approved_count,
            self.total_advisors,
            self.highest_risk,
            self.confidence * 100.0
        )
    }

    /// Formats the consensus resolution as a Markdown report.
    pub fn to_markdown(&self) -> String {
        let mut out = String::new();
        let status_str = if self.approved {
            "🟢 **APPROVED**"
        } else {
            "🔴 **REJECTED**"
        };

        out.push_str(&format!(
            "### Advisor Consensus: {}\n\n",
            status_str
        ));
        out.push_str(&format!("- **Strategy**: {}\n", self.strategy));
        out.push_str(&format!(
            "- **Vote Count**: {} / {} approved\n",
            self.approved_count, self.total_advisors
        ));
        out.push_str(&format!(
            "- **Confidence**: {:.1}%\n",
            self.confidence * 100.0
        ));
        out.push_str(&format!(
            "- **Highest Risk**: `{}` (Aggregate: {:.1}/10.0)\n",
            self.highest_risk, self.aggregate_risk_score
        ));
        out.push_str(&format!("- **Summary**: {}\n", self.summary));

        if !self.veto_reasons.is_empty() {
            out.push_str("\n#### ⚠️ Veto Details\n");
            for reason in &self.veto_reasons {
                out.push_str(&format!("- {}\n", reason));
            }
        }

        if !self.votes.is_empty() {
            out.push_str("\n#### Advisor Breakdown\n");
            for vote in &self.votes {
                let vote_icon = if vote.vetoed {
                    "🚫 VETO"
                } else if vote.approved {
                    "✅ Approve"
                } else {
                    "❌ Disapprove"
                };
                out.push_str(&format!(
                    "- **{}** ({}): {} | Risk: `{}` | Weight: {:.1}\n  > {}\n",
                    vote.advisor,
                    vote.focus,
                    vote_icon,
                    vote.risk_level,
                    vote.effective_weight,
                    vote.critique
                ));
            }
        }

        if !self.recommendations.is_empty() {
            out.push_str("\n#### Collective Recommendations\n");
            for rec in &self.recommendations {
                out.push_str(&format!("- {}\n", rec));
            }
        }

        out
    }
}

/// Configurable policy controlling consensus parameters and risk tolerances.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConsensusPolicy {
    /// Consensus algorithm to use.
    pub strategy: ConsensusStrategy,
    /// Automatically veto if any advisor flags a Critical risk.
    pub critical_auto_veto: bool,
    /// Automatically veto if any advisor flags a High risk.
    pub high_risk_auto_veto: bool,
    /// Minimum confidence threshold (0.0 to 1.0) required for approval.
    pub min_confidence_threshold: f64,
    /// Minimum quorum of advisor reviews required (0 = no minimum).
    pub min_quorum: usize,
    /// Base weights assigned per risk level.
    pub risk_weights: HashMap<RiskLevel, f64>,
    /// Custom weights per advisor name.
    pub advisor_weights: HashMap<String, f64>,
    /// Whether an empty list of critiques is approved by default.
    pub allow_empty_approval: bool,
}

impl Default for ConsensusPolicy {
    fn default() -> Self {
        let mut risk_weights = HashMap::new();
        risk_weights.insert(RiskLevel::Low, 1.0);
        risk_weights.insert(RiskLevel::Medium, 2.0);
        risk_weights.insert(RiskLevel::High, 4.0);
        risk_weights.insert(RiskLevel::Critical, 8.0);

        Self {
            strategy: ConsensusStrategy::RiskWeighted,
            critical_auto_veto: true,
            high_risk_auto_veto: false,
            min_confidence_threshold: 0.50,
            min_quorum: 1,
            risk_weights,
            advisor_weights: HashMap::new(),
            allow_empty_approval: true,
        }
    }
}

impl ConsensusPolicy {
    /// Creates a new policy with the specified strategy.
    pub fn new(strategy: ConsensusStrategy) -> Self {
        Self {
            strategy,
            ..Default::default()
        }
    }

    /// Strict unanimous policy: all advisors must approve, critical risks veto.
    pub fn unanimous() -> Self {
        Self::new(ConsensusStrategy::Unanimous)
    }

    /// Standard majority policy (> 50% approval).
    pub fn majority() -> Self {
        Self::new(ConsensusStrategy::Majority)
    }

    /// Risk-weighted policy with exponential risk penalties.
    pub fn risk_weighted() -> Self {
        Self::new(ConsensusStrategy::RiskWeighted)
    }

    /// Security-first veto policy.
    pub fn security_veto() -> Self {
        Self::new(ConsensusStrategy::SecurityVeto)
    }

    /// Supermajority policy with custom threshold (e.g. 0.67 for 2/3).
    pub fn supermajority(threshold: f64) -> Self {
        Self::new(ConsensusStrategy::Supermajority(threshold.clamp(0.0, 1.0)))
    }

    /// Builder method to set critical auto-veto.
    pub fn with_critical_auto_veto(mut self, enabled: bool) -> Self {
        self.critical_auto_veto = enabled;
        self
    }

    /// Builder method to set high-risk auto-veto.
    pub fn with_high_risk_auto_veto(mut self, enabled: bool) -> Self {
        self.high_risk_auto_veto = enabled;
        self
    }

    /// Builder method to set the minimum confidence threshold.
    pub fn with_min_confidence(mut self, threshold: f64) -> Self {
        self.min_confidence_threshold = threshold.clamp(0.0, 1.0);
        self
    }

    /// Builder method to set custom advisor weight.
    pub fn with_advisor_weight(mut self, advisor_name: impl Into<String>, weight: f64) -> Self {
        self.advisor_weights.insert(advisor_name.into(), weight.max(0.0));
        self
    }
}

/// Consensus Engine that resolves advisor critiques into a collective decision.
#[derive(Debug, Clone)]
pub struct ConsensusEngine {
    policy: ConsensusPolicy,
}

impl Default for ConsensusEngine {
    fn default() -> Self {
        Self::new(ConsensusPolicy::default())
    }
}

impl ConsensusEngine {
    /// Creates a new ConsensusEngine with the given policy.
    pub fn new(policy: ConsensusPolicy) -> Self {
        Self { policy }
    }

    /// Creates a new ConsensusEngine with a specific strategy and default settings.
    pub fn with_strategy(strategy: ConsensusStrategy) -> Self {
        Self::new(ConsensusPolicy::new(strategy))
    }

    /// Returns a reference to the active policy.
    pub fn policy(&self) -> &ConsensusPolicy {
        &self.policy
    }

    /// Sets or updates the active policy.
    pub fn set_policy(&mut self, policy: ConsensusPolicy) {
        self.policy = policy;
    }

    /// Resolves an array of advisor critiques using the configured policy.
    pub fn resolve(&self, critiques: &[AdvisorCritique]) -> ConsensusResolution {
        resolve_consensus_with_policy(critiques, &self.policy)
    }

    /// Resolves advisor critiques using a specific override strategy.
    pub fn resolve_with_strategy(
        &self,
        critiques: &[AdvisorCritique],
        strategy: ConsensusStrategy,
    ) -> ConsensusResolution {
        let mut custom_policy = self.policy.clone();
        custom_policy.strategy = strategy;
        resolve_consensus_with_policy(critiques, &custom_policy)
    }

    /// Resolves using Majority voting.
    pub fn resolve_majority(&self, critiques: &[AdvisorCritique]) -> ConsensusResolution {
        self.resolve_with_strategy(critiques, ConsensusStrategy::Majority)
    }

    /// Resolves using Unanimous consensus.
    pub fn resolve_unanimous(&self, critiques: &[AdvisorCritique]) -> ConsensusResolution {
        self.resolve_with_strategy(critiques, ConsensusStrategy::Unanimous)
    }

    /// Resolves using Risk-Weighted voting.
    pub fn resolve_risk_weighted(&self, critiques: &[AdvisorCritique]) -> ConsensusResolution {
        self.resolve_with_strategy(critiques, ConsensusStrategy::RiskWeighted)
    }

    /// Resolves using Supermajority voting with the given threshold fraction.
    pub fn resolve_supermajority(
        &self,
        critiques: &[AdvisorCritique],
        threshold: f64,
    ) -> ConsensusResolution {
        self.resolve_with_strategy(critiques, ConsensusStrategy::Supermajority(threshold.clamp(0.0, 1.0)))
    }

    /// Resolves using Security-First Veto voting.
    pub fn resolve_security_veto(&self, critiques: &[AdvisorCritique]) -> ConsensusResolution {
        self.resolve_with_strategy(critiques, ConsensusStrategy::SecurityVeto)
    }
}

// ============================================================================
// Core Consensus Resolution Logic
// ============================================================================

/// Evaluates advisor critiques using the specified strategy and default policy options.
pub fn resolve_consensus(
    critiques: &[AdvisorCritique],
    strategy: ConsensusStrategy,
) -> ConsensusResolution {
    let policy = ConsensusPolicy::new(strategy);
    resolve_consensus_with_policy(critiques, &policy)
}

/// Evaluates advisor critiques using Majority voting.
pub fn resolve_majority(critiques: &[AdvisorCritique]) -> ConsensusResolution {
    resolve_consensus(critiques, ConsensusStrategy::Majority)
}

/// Evaluates advisor critiques using Unanimous voting.
pub fn resolve_unanimous(critiques: &[AdvisorCritique]) -> ConsensusResolution {
    resolve_consensus(critiques, ConsensusStrategy::Unanimous)
}

/// Evaluates advisor critiques using Risk-Weighted voting.
pub fn resolve_risk_weighted(critiques: &[AdvisorCritique]) -> ConsensusResolution {
    resolve_consensus(critiques, ConsensusStrategy::RiskWeighted)
}

/// Evaluates advisor critiques using Security-First Veto voting.
pub fn resolve_security_veto(critiques: &[AdvisorCritique]) -> ConsensusResolution {
    resolve_consensus(critiques, ConsensusStrategy::SecurityVeto)
}

/// Resolves advisor critiques according to a comprehensive `ConsensusPolicy`.
pub fn resolve_consensus_with_policy(
    critiques: &[AdvisorCritique],
    policy: &ConsensusPolicy,
) -> ConsensusResolution {
    let total_advisors = critiques.len();

    // 1. Handle empty reviews
    if total_advisors == 0 {
        let approved = policy.allow_empty_approval;
        let summary = if approved {
            "No advisor critiques provided; auto-approved by policy.".to_string()
        } else {
            "No advisor critiques provided; rejected due to zero reviews.".to_string()
        };

        return ConsensusResolution {
            approved,
            strategy: policy.strategy.clone(),
            confidence: if approved { 1.0 } else { 0.0 },
            total_advisors: 0,
            approved_count: 0,
            rejected_count: 0,
            highest_risk: RiskLevel::Low,
            aggregate_risk_score: 0.0,
            summary,
            veto_reasons: Vec::new(),
            dissenting_critiques: Vec::new(),
            recommendations: Vec::new(),
            votes: Vec::new(),
        };
    }

    // 2. Aggregate baseline metrics
    let mut approved_count = 0;
    let mut rejected_count = 0;
    let mut highest_risk = RiskLevel::Low;
    let mut veto_reasons = Vec::new();
    let mut dissenting_critiques = Vec::new();
    let mut raw_suggestions = Vec::new();
    let mut total_risk_points: f64 = 0.0;
    let mut peak_risk_points: f64 = 0.0;

    for critique in critiques {
        if critique.approved {
            approved_count += 1;
        } else {
            rejected_count += 1;
            dissenting_critiques.push(critique.clone());
        }

        if critique.risk_level > highest_risk {
            highest_risk = critique.risk_level;
        }

        let risk_points = match critique.risk_level {
            RiskLevel::Low => 1.0,
            RiskLevel::Medium => 3.0,
            RiskLevel::High => 6.0,
            RiskLevel::Critical => 10.0,
        };
        peak_risk_points = peak_risk_points.max(risk_points);
        total_risk_points += risk_points;

        for s in &critique.suggestions {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                raw_suggestions.push(trimmed.to_string());
            }
        }
    }

    // Deduplicate suggestions while preserving order
    let mut seen_suggestions = HashSet::new();
    let mut recommendations = Vec::new();
    for sugg in raw_suggestions {
        if seen_suggestions.insert(sugg.clone()) {
            recommendations.push(sugg);
        }
    }

    // Blends the mean advisor risk with the peak assessed risk
    // (0.6 * mean + 0.4 * peak) so a single severe dissent cannot be diluted
    // by a large pool of low-risk approvals. Scale: 0.0 (no risk) - 10.0.
    let mean_risk_points = total_risk_points / total_advisors as f64;
    let aggregate_risk_score: f64 =
        (0.6 * mean_risk_points + 0.4 * peak_risk_points).clamp(0.0, 10.0);

    // 3. Check hard global auto-vetoes
    if policy.critical_auto_veto && highest_risk == RiskLevel::Critical {
        for c in critiques.iter().filter(|c| c.risk_level == RiskLevel::Critical) {
            veto_reasons.push(format!(
                "Critical Risk Veto by {}: {}",
                c.advisor, c.critique
            ));
        }
    }

    if policy.high_risk_auto_veto && highest_risk >= RiskLevel::High {
        for c in critiques.iter().filter(|c| c.risk_level >= RiskLevel::High) {
            veto_reasons.push(format!(
                "High Risk Policy Veto by {} (risk: {}): {}",
                c.advisor, c.risk_level, c.critique
            ));
        }
    }

    // 4. Check quorum requirement
    if policy.min_quorum > 0 && total_advisors < policy.min_quorum {
        veto_reasons.push(format!(
            "Quorum not reached: required {} advisors, received {}",
            policy.min_quorum, total_advisors
        ));
    }

    // 5. Evaluate strategy-specific votes and determine approval & confidence
    let (mut strategy_approved, confidence, votes) = match &policy.strategy {
        ConsensusStrategy::Majority => {
            evaluate_majority_strategy(critiques, policy)
        }
        ConsensusStrategy::Unanimous => {
            evaluate_unanimous_strategy(critiques, policy, &mut veto_reasons)
        }
        ConsensusStrategy::RiskWeighted => {
            evaluate_risk_weighted_strategy(critiques, policy, &mut veto_reasons)
        }
        ConsensusStrategy::Supermajority(threshold) => {
            evaluate_supermajority_strategy(critiques, *threshold, policy)
        }
        ConsensusStrategy::SecurityVeto => {
            evaluate_security_veto_strategy(critiques, policy, &mut veto_reasons)
        }
        ConsensusStrategy::CustomWeighted { weights, default_weight } => {
            evaluate_custom_weighted_strategy(critiques, weights, *default_weight, policy, &mut veto_reasons)
        }
    };

    // 6. Confidence threshold check
    if strategy_approved && confidence < policy.min_confidence_threshold {
        strategy_approved = false;
        veto_reasons.push(format!(
            "Confidence {:.1}% fell below required policy threshold of {:.1}%",
            confidence * 100.0,
            policy.min_confidence_threshold * 100.0
        ));
    }

    // 7. Final approval determination (vetoes unconditionally override strategy approval)
    let final_approved = strategy_approved && veto_reasons.is_empty();

    // 8. Generate concise, transparent consensus summary
    let summary = generate_consensus_summary(
        final_approved,
        &policy.strategy,
        approved_count,
        total_advisors,
        highest_risk,
        confidence,
        &veto_reasons,
    );

    ConsensusResolution {
        approved: final_approved,
        strategy: policy.strategy.clone(),
        confidence,
        total_advisors,
        approved_count,
        rejected_count,
        highest_risk,
        aggregate_risk_score,
        summary,
        veto_reasons,
        dissenting_critiques,
        recommendations,
        votes,
    }
}

// ============================================================================
// Strategy Evaluation Functions
// ============================================================================

/// Evaluates critiques using standard Majority voting (> 50%).
fn evaluate_majority_strategy(
    critiques: &[AdvisorCritique],
    policy: &ConsensusPolicy,
) -> (bool, f64, Vec<AdvisorVote>) {
    let total = critiques.len();
    let mut approved_count = 0;
    let mut votes = Vec::with_capacity(total);

    for c in critiques {
        let weight = get_effective_advisor_weight(c, policy);
        let score = if c.approved {
            approved_count += 1;
            weight
        } else {
            -weight
        };

        votes.push(AdvisorVote {
            advisor: c.advisor.clone(),
            focus: c.focus.clone(),
            approved: c.approved,
            risk_level: c.risk_level,
            effective_weight: weight,
            score_contribution: score,
            vetoed: false,
            critique: c.critique.clone(),
            suggestions: c.suggestions.clone(),
        });
    }

    let approval_fraction = approved_count as f64 / total as f64;
    // Strict majority: approved_count must be strictly greater than half
    let is_approved = approved_count > (total / 2);
    let confidence = approval_fraction;

    (is_approved, confidence, votes)
}

/// Evaluates critiques using Supermajority voting (e.g. >= threshold fraction).
fn evaluate_supermajority_strategy(
    critiques: &[AdvisorCritique],
    threshold: f64,
    policy: &ConsensusPolicy,
) -> (bool, f64, Vec<AdvisorVote>) {
    let total = critiques.len();
    let mut approved_count = 0;
    let mut votes = Vec::with_capacity(total);

    for c in critiques {
        let weight = get_effective_advisor_weight(c, policy);
        let score = if c.approved {
            approved_count += 1;
            weight
        } else {
            -weight
        };

        votes.push(AdvisorVote {
            advisor: c.advisor.clone(),
            focus: c.focus.clone(),
            approved: c.approved,
            risk_level: c.risk_level,
            effective_weight: weight,
            score_contribution: score,
            vetoed: false,
            critique: c.critique.clone(),
            suggestions: c.suggestions.clone(),
        });
    }

    let approval_fraction = approved_count as f64 / total as f64;
    let is_approved = approval_fraction >= threshold;
    let confidence = approval_fraction;

    (is_approved, confidence, votes)
}

/// Evaluates critiques using Unanimous consensus (100% approval required).
fn evaluate_unanimous_strategy(
    critiques: &[AdvisorCritique],
    policy: &ConsensusPolicy,
    veto_reasons: &mut Vec<String>,
) -> (bool, f64, Vec<AdvisorVote>) {
    let total = critiques.len();
    let mut approved_count = 0;
    let mut votes = Vec::with_capacity(total);

    for c in critiques {
        let weight = get_effective_advisor_weight(c, policy);
        let vetoed = !c.approved;

        if vetoed {
            veto_reasons.push(format!(
                "Unanimous consensus broken by {}: {}",
                c.advisor, c.critique
            ));
        }

        let score = if c.approved {
            approved_count += 1;
            weight
        } else {
            -weight
        };

        votes.push(AdvisorVote {
            advisor: c.advisor.clone(),
            focus: c.focus.clone(),
            approved: c.approved,
            risk_level: c.risk_level,
            effective_weight: weight,
            score_contribution: score,
            vetoed,
            critique: c.critique.clone(),
            suggestions: c.suggestions.clone(),
        });
    }

    let is_approved = approved_count == total;
    let confidence = if is_approved { 1.0 } else { approved_count as f64 / total as f64 };

    (is_approved, confidence, votes)
}

/// Evaluates critiques using Risk-Weighted voting with exponential risk impact.
fn evaluate_risk_weighted_strategy(
    critiques: &[AdvisorCritique],
    policy: &ConsensusPolicy,
    _veto_reasons: &mut Vec<String>,
) -> (bool, f64, Vec<AdvisorVote>) {
    let total = critiques.len();
    let mut positive_weight = 0.0;
    let mut negative_weight = 0.0;
    let mut total_weight = 0.0;
    let mut max_risk_penalty = 0.0;
    let mut votes = Vec::with_capacity(total);

    for c in critiques {
        let base_advisor_weight = policy
            .advisor_weights
            .get(&c.advisor)
            .copied()
            .unwrap_or(1.0);

        let risk_multiplier = policy
            .risk_weights
            .get(&c.risk_level)
            .copied()
            .unwrap_or_else(|| match c.risk_level {
                RiskLevel::Low => 1.0,
                RiskLevel::Medium => 2.0,
                RiskLevel::High => 4.0,
                RiskLevel::Critical => 8.0,
            });

        let effective_weight = base_advisor_weight * risk_multiplier;
        total_weight += effective_weight;

        // Risk penalty factor
        let risk_factor = match c.risk_level {
            RiskLevel::Low => 0.05,
            RiskLevel::Medium => 0.20,
            RiskLevel::High => 0.50,
            RiskLevel::Critical => 0.90,
        };
        if risk_factor > max_risk_penalty {
            max_risk_penalty = risk_factor;
        }

        let score = if c.approved {
            positive_weight += effective_weight;
            effective_weight * (1.0 - risk_factor)
        } else {
            negative_weight += effective_weight;
            -effective_weight * (1.0 + risk_factor)
        };

        votes.push(AdvisorVote {
            advisor: c.advisor.clone(),
            focus: c.focus.clone(),
            approved: c.approved,
            risk_level: c.risk_level,
            effective_weight,
            score_contribution: score,
            vetoed: false,
            critique: c.critique.clone(),
            suggestions: c.suggestions.clone(),
        });
    }

    let raw_ratio = if total_weight > 0.0 {
        positive_weight / total_weight
    } else {
        0.0
    };

    // Confidence decays if high risks are present
    let critical_scale = policy
        .risk_weights
        .get(&RiskLevel::Critical)
        .copied()
        .unwrap_or(8.0)
        / 8.0;
    let confidence = (raw_ratio * (1.0 - (max_risk_penalty * critical_scale * 0.5))).clamp(0.0, 1.0);
    // Approval requires positive weight to strictly exceed negative weight
    // and confidence to reach the 0.5 barrier.
    let is_approved = positive_weight > negative_weight && confidence >= 0.50;

    (is_approved, confidence, votes)
}

/// Evaluates critiques using Security-First Veto strategy.
fn evaluate_security_veto_strategy(
    critiques: &[AdvisorCritique],
    policy: &ConsensusPolicy,
    veto_reasons: &mut Vec<String>,
) -> (bool, f64, Vec<AdvisorVote>) {
    let total = critiques.len();
    let mut approved_count = 0;
    let mut security_veto = false;
    let mut votes = Vec::with_capacity(total);

    for c in critiques {
        let is_security_advisor = c.advisor.to_lowercase().contains("security");
        let weight = get_effective_advisor_weight(c, policy);
        let mut vetoed = false;

        // Security advisor rejects or assesses High/Critical risk -> immediate veto
        if is_security_advisor && (!c.approved || c.risk_level >= RiskLevel::High) {
            security_veto = true;
            vetoed = true;
            veto_reasons.push(format!(
                "Security Advisor Veto (risk: {}): {}",
                c.risk_level, c.critique
            ));
        }

        let score = if c.approved {
            approved_count += 1;
            weight
        } else {
            -weight
        };

        votes.push(AdvisorVote {
            advisor: c.advisor.clone(),
            focus: c.focus.clone(),
            approved: c.approved,
            risk_level: c.risk_level,
            effective_weight: weight,
            score_contribution: score,
            vetoed,
            critique: c.critique.clone(),
            suggestions: c.suggestions.clone(),
        });
    }

    let approval_fraction = approved_count as f64 / total as f64;
    let is_majority = approved_count > (total / 2);
    let is_approved = !security_veto && is_majority;
    let confidence = if security_veto { 0.0 } else { approval_fraction };

    (is_approved, confidence, votes)
}

/// Evaluates critiques using custom specified advisor weights.
fn evaluate_custom_weighted_strategy(
    critiques: &[AdvisorCritique],
    weights: &HashMap<String, f64>,
    default_weight: f64,
    _policy: &ConsensusPolicy,
    _veto_reasons: &mut Vec<String>,
) -> (bool, f64, Vec<AdvisorVote>) {
    let total = critiques.len();
    let mut positive_weight = 0.0;
    let mut negative_weight = 0.0;
    let mut total_weight = 0.0;
    let mut votes = Vec::with_capacity(total);

    for c in critiques {
        let weight = weights
            .get(&c.advisor)
            .copied()
            .unwrap_or(default_weight)
            .max(0.0);

        total_weight += weight;

        let score = if c.approved {
            positive_weight += weight;
            weight
        } else {
            negative_weight += weight;
            -weight
        };

        votes.push(AdvisorVote {
            advisor: c.advisor.clone(),
            focus: c.focus.clone(),
            approved: c.approved,
            risk_level: c.risk_level,
            effective_weight: weight,
            score_contribution: score,
            vetoed: false,
            critique: c.critique.clone(),
            suggestions: c.suggestions.clone(),
        });
    }

    let raw_ratio = if total_weight > 0.0 {
        positive_weight / total_weight
    } else {
        0.0
    };

    // Confidence equals the weighted approval ratio; custom weights make
    // dominant approvers decisive while minor reviewers remain advisory.
    let confidence = raw_ratio.clamp(0.0, 1.0);

    // Approval requires positive weight to strictly exceed negative weight.
    let is_approved = positive_weight > negative_weight && confidence >= 0.50;

    (is_approved, confidence, votes)
}

/// Helper to get effective advisor weight from policy.
fn get_effective_advisor_weight(critique: &AdvisorCritique, policy: &ConsensusPolicy) -> f64 {
    policy
        .advisor_weights
        .get(&critique.advisor)
        .copied()
        .unwrap_or(1.0)
}

/// Generates a human-readable summary string for the consensus resolution.
fn generate_consensus_summary(
    approved: bool,
    strategy: &ConsensusStrategy,
    approved_count: usize,
    total_advisors: usize,
    highest_risk: RiskLevel,
    confidence: f64,
    veto_reasons: &[String],
) -> String {
    if !veto_reasons.is_empty() {
        return format!(
            "Consensus REJECTED via {} ({}): {}",
            strategy,
            highest_risk,
            veto_reasons.join("; ")
        );
    }

    if approved {
        format!(
            "Consensus APPROVED via {} ({}/{} approved, confidence: {:.0}%, highest risk: {})",
            strategy,
            approved_count,
            total_advisors,
            confidence * 100.0,
            highest_risk
        )
    } else {
        format!(
            "Consensus REJECTED via {} ({}/{} approved, confidence: {:.0}%, highest risk: {})",
            strategy,
            approved_count,
            total_advisors,
            confidence * 100.0,
            highest_risk
        )
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_critique(advisor: &str, approved: bool, risk: RiskLevel, text: &str) -> AdvisorCritique {
        AdvisorCritique {
            advisor: advisor.to_string(),
            focus: format!("{} focus", advisor),
            approved,
            risk_level: risk,
            critique: text.to_string(),
            suggestions: vec![format!("Suggestion from {}", advisor)],
        }
    }

    #[test]
    fn test_empty_critiques_resolution() {
        let policy = ConsensusPolicy::default();
        let resolution = resolve_consensus_with_policy(&[], &policy);
        assert!(resolution.is_approved());
        assert_eq!(resolution.total_advisors, 0);
        assert_eq!(resolution.confidence, 1.0);

        let strict_policy = ConsensusPolicy {
            allow_empty_approval: false,
            ..Default::default()
        };
        let resolution_strict = resolve_consensus_with_policy(&[], &strict_policy);
        assert!(!resolution_strict.is_approved());
        assert_eq!(resolution_strict.confidence, 0.0);
    }

    #[test]
    fn test_majority_voting() {
        // 2 approve, 1 reject -> Approved
        let critiques = vec![
            sample_critique("ArchitectureAdvisor", true, RiskLevel::Low, "Good modular structure"),
            sample_critique("SecurityAdvisor", true, RiskLevel::Low, "Safe operations"),
            sample_critique("CodeReviewAdvisor", false, RiskLevel::Medium, "Needs minor formatting"),
        ];

        let resolution = resolve_majority(&critiques);
        assert!(resolution.is_approved());
        assert_eq!(resolution.approved_count, 2);
        assert_eq!(resolution.rejected_count, 1);
        assert_eq!(resolution.highest_risk, RiskLevel::Medium);
        assert_eq!(resolution.dissenting_critiques.len(), 1);
        assert_eq!(resolution.recommendations.len(), 3);

        // 1 approve, 2 reject -> Rejected
        let critiques_rejected = vec![
            sample_critique("ArchitectureAdvisor", true, RiskLevel::Low, "Fine"),
            sample_critique("SecurityAdvisor", false, RiskLevel::High, "Command injection risk"),
            sample_critique("CodeReviewAdvisor", false, RiskLevel::Medium, "Poor error handling"),
        ];

        let res_rejected = resolve_majority(&critiques_rejected);
        assert!(!res_rejected.is_approved());
        assert_eq!(res_rejected.approved_count, 1);
        assert_eq!(res_rejected.rejected_count, 2);
    }

    #[test]
    fn test_majority_tie_is_rejected() {
        let critiques = vec![
            sample_critique("ArchitectureAdvisor", true, RiskLevel::Low, "Looks good"),
            sample_critique("SecurityAdvisor", false, RiskLevel::Medium, "Concern"),
        ];

        let resolution = resolve_majority(&critiques);
        assert!(!resolution.is_approved());
        assert_eq!(resolution.approved_count, 1);
        assert_eq!(resolution.rejected_count, 1);
    }

    #[test]
    fn test_unanimous_voting() {
        // 3 approve -> Approved
        let critiques_approved = vec![
            sample_critique("ArchitectureAdvisor", true, RiskLevel::Low, "Approved"),
            sample_critique("SecurityAdvisor", true, RiskLevel::Low, "Approved"),
            sample_critique("CodeReviewAdvisor", true, RiskLevel::Low, "Approved"),
        ];

        let res = resolve_unanimous(&critiques_approved);
        assert!(res.is_approved());
        assert_eq!(res.confidence, 1.0);
        assert!(res.veto_reasons.is_empty());

        // 2 approve, 1 reject -> Rejected with veto reason
        let critiques_rejected = vec![
            sample_critique("ArchitectureAdvisor", true, RiskLevel::Low, "Approved"),
            sample_critique("SecurityAdvisor", false, RiskLevel::Low, "Needs token limit"),
            sample_critique("CodeReviewAdvisor", true, RiskLevel::Low, "Approved"),
        ];

        let res_rej = resolve_unanimous(&critiques_rejected);
        assert!(!res_rej.is_approved());
        assert_eq!(res_rej.approved_count, 2);
        assert_eq!(res_rej.rejected_count, 1);
        assert!(!res_rej.veto_reasons.is_empty());
        assert!(res_rej.veto_reasons[0].contains("SecurityAdvisor"));
    }

    #[test]
    fn test_risk_weighted_voting() {
        // 2 Low risk approvals vs 1 High risk rejection -> Rejected due to heavy high-risk weight
        let critiques = vec![
            sample_critique("ArchitectureAdvisor", true, RiskLevel::Low, "Architecturally fine"),
            sample_critique("CodeReviewAdvisor", true, RiskLevel::Low, "Code is idiomatic"),
            sample_critique("SecurityAdvisor", false, RiskLevel::High, "Exposes internal credentials"),
        ];

        let resolution = resolve_risk_weighted(&critiques);
        assert!(!resolution.is_approved());
        assert_eq!(resolution.highest_risk, RiskLevel::High);

        // 2 Medium risk approvals vs 1 Low risk rejection -> Approved with confidence
        let critiques_app = vec![
            sample_critique("ArchitectureAdvisor", true, RiskLevel::Medium, "Acceptable coupling"),
            sample_critique("SecurityAdvisor", true, RiskLevel::Medium, "Safe with caveats"),
            sample_critique("CodeReviewAdvisor", false, RiskLevel::Low, "Nit: comment typo"),
        ];

        let res_app = resolve_risk_weighted(&critiques_app);
        assert!(res_app.is_approved());
        assert_eq!(res_app.approved_count, 2);
    }

    #[test]
    fn test_critical_risk_auto_veto() {
        // Even with Majority strategy, a Critical risk triggers auto-veto by default policy
        let critiques = vec![
            sample_critique("ArchitectureAdvisor", true, RiskLevel::Low, "Fine"),
            sample_critique("CodeReviewAdvisor", true, RiskLevel::Low, "Fine"),
            sample_critique("SecurityAdvisor", false, RiskLevel::Critical, "Command executes `rm -rf /`"),
        ];

        let resolution = resolve_majority(&critiques);
        assert!(!resolution.is_approved());
        assert_eq!(resolution.highest_risk, RiskLevel::Critical);
        assert!(resolution.has_veto());
        assert!(resolution.veto_reasons[0].contains("Critical Risk Veto"));
    }

    #[test]
    fn test_security_veto_strategy() {
        // Non-security disapproval does not trigger veto; decided by majority
        let critiques_non_sec_disapprove = vec![
            sample_critique("SecurityAdvisor", true, RiskLevel::Low, "Safe to run"),
            sample_critique("ArchitectureAdvisor", true, RiskLevel::Low, "Modular"),
            sample_critique("CodeReviewAdvisor", false, RiskLevel::Low, "Minor nit"),
        ];

        let res1 = resolve_security_veto(&critiques_non_sec_disapprove);
        assert!(res1.is_approved());
        assert_eq!(res1.approved_count, 2);

        // Security disapproval triggers immediate veto
        let critiques_sec_disapprove = vec![
            sample_critique("SecurityAdvisor", false, RiskLevel::Medium, "Potential directory traversal"),
            sample_critique("ArchitectureAdvisor", true, RiskLevel::Low, "Modular"),
            sample_critique("CodeReviewAdvisor", true, RiskLevel::Low, "Looks fine"),
        ];

        let res2 = resolve_security_veto(&critiques_sec_disapprove);
        assert!(!res2.is_approved());
        assert!(res2.has_veto());
        assert!(res2.veto_reasons[0].contains("Security Advisor Veto"));
    }

    #[test]
    fn test_supermajority_strategy() {
        // 2 out of 3 = 66.7%
        let critiques = vec![
            sample_critique("ArchitectureAdvisor", true, RiskLevel::Low, "OK"),
            sample_critique("CodeReviewAdvisor", true, RiskLevel::Low, "OK"),
            sample_critique("SecurityAdvisor", false, RiskLevel::Low, "Needs tweak"),
        ];

        // 60% threshold -> passes (66.7% >= 60%)
        let policy_60 = ConsensusPolicy::supermajority(0.60);
        let res_60 = resolve_consensus_with_policy(&critiques, &policy_60);
        assert!(res_60.is_approved());

        // 75% threshold -> fails (66.7% < 75%)
        let policy_75 = ConsensusPolicy::supermajority(0.75);
        let res_75 = resolve_consensus_with_policy(&critiques, &policy_75);
        assert!(!res_75.is_approved());
    }

    #[test]
    fn test_custom_weighted_strategy() {
        let mut weights = HashMap::new();
        weights.insert("LeadArchitect".to_string(), 5.0);
        weights.insert("JuniorReviewer".to_string(), 1.0);

        let critiques = vec![
            sample_critique("LeadArchitect", true, RiskLevel::Low, "Ship it"),
            sample_critique("JuniorReviewer", false, RiskLevel::Low, "Unsure"),
        ];

        let strategy = ConsensusStrategy::CustomWeighted {
            weights,
            default_weight: 1.0,
        };
        let policy = ConsensusPolicy::new(strategy);
        let res = resolve_consensus_with_policy(&critiques, &policy);
        assert!(res.is_approved());
        assert_eq!(res.votes[0].effective_weight, 5.0);
        assert_eq!(res.votes[1].effective_weight, 1.0);
    }

    #[test]
    fn test_consensus_engine_and_formatting() {
        let engine = ConsensusEngine::with_strategy(ConsensusStrategy::Majority);
        let critiques = vec![
            sample_critique("SecurityAdvisor", true, RiskLevel::Low, "No vulnerabilities found"),
            sample_critique("ArchitectureAdvisor", true, RiskLevel::Low, "Clean separation of concerns"),
        ];

        let resolution = engine.resolve(&critiques);
        assert!(resolution.is_approved());

        let badge = resolution.status_badge();
        assert!(badge.contains("✓ APPROVED"));
        assert!(badge.contains("2/2 approved"));

        let md = resolution.to_markdown();
        assert!(md.contains("### Advisor Consensus: 🟢 **APPROVED**"));
        assert!(md.contains("SecurityAdvisor"));
        assert!(md.contains("Collective Recommendations"));
    }

    #[test]
    fn test_aggregate_risk_score_blends_mean_and_peak() {
        // All Low (1.0): mean 1.0, peak 1.0 -> 0.6*1.0 + 0.4*1.0 = 1.0
        let critiques_low = vec![
            sample_critique("ArchitectureAdvisor", true, RiskLevel::Low, "Fine"),
            sample_critique("SecurityAdvisor", true, RiskLevel::Low, "Fine"),
        ];
        let res_low = resolve_majority(&critiques_low);
        assert!((res_low.aggregate_risk_score - 1.0).abs() < 1e-9);

        // Single Critical dissent (10.0) among Lows: mean (1+1+10)/3 = 4.0,
        // peak 10.0 -> 0.6*4.0 + 0.4*10.0 = 6.4 (mean alone would dilute it).
        let critiques_peak = vec![
            sample_critique("ArchitectureAdvisor", true, RiskLevel::Low, "Fine"),
            sample_critique("SecurityAdvisor", true, RiskLevel::Low, "Fine"),
            sample_critique("CodeReviewAdvisor", false, RiskLevel::Critical, "Deletes production data"),
        ];
        let res_peak = resolve_majority(&critiques_peak);
        assert!((res_peak.aggregate_risk_score - 6.4).abs() < 1e-9);
        assert!((res_peak.aggregate_risk_score - 4.0).abs() > 1e-9);
    }

    #[test]
    fn test_critical_veto_overrides_strategy_approval() {
        // 2 approve, 1 reject, but the dissenter flags Critical -> auto-veto
        // unconditionally overrides strategy approval.
        let critiques = vec![
            sample_critique("ArchitectureAdvisor", true, RiskLevel::Low, "OK"),
            sample_critique("CodeReviewAdvisor", true, RiskLevel::Low, "OK"),
            sample_critique("SecurityAdvisor", false, RiskLevel::Critical, "Executes arbitrary shell"),
        ];
        let res = resolve_majority(&critiques);
        assert_eq!(res.approved_count, 2);
        assert_eq!(res.rejected_count, 1);
        assert!(!res.is_approved());
        assert!(res.has_veto());
        assert!(res.is_critical());
        assert!(res.is_high_or_critical());
    }

    #[test]
    fn test_high_risk_auto_veto_policy() {
        let policy = ConsensusPolicy::majority().with_high_risk_auto_veto(true);
        let critiques = vec![
            sample_critique("ArchitectureAdvisor", true, RiskLevel::Low, "OK"),
            sample_critique("CodeReviewAdvisor", true, RiskLevel::Low, "OK"),
            sample_critique("SecurityAdvisor", false, RiskLevel::High, "Secret exposure"),
        ];
        let res = resolve_consensus_with_policy(&critiques, &policy);
        assert!(!res.is_approved());
        assert!(res.has_veto());
        assert!(res.veto_reasons.iter().any(|r| r.contains("High Risk Policy Veto")));
    }

    #[test]
    fn test_quorum_not_reached() {
        let policy = ConsensusPolicy {
            min_quorum: 3,
            ..ConsensusPolicy::majority()
        };
        let critiques = vec![
            sample_critique("ArchitectureAdvisor", true, RiskLevel::Low, "OK"),
        ];
        let res = resolve_consensus_with_policy(&critiques, &policy);
        assert!(!res.is_approved());
        assert!(res.has_veto());
        assert!(res.veto_reasons.iter().any(|r| r.contains("Quorum not reached")));
    }

    #[test]
    fn test_confidence_threshold_policy() {
        // Supermajority 0.51 with min confidence 0.90: a bare 51% approval
        // passes the strategy but fails the confidence gate.
        let policy = ConsensusPolicy::supermajority(0.51).with_min_confidence(0.90);
        let critiques = vec![
            sample_critique("ArchitectureAdvisor", true, RiskLevel::Low, "OK"),
            sample_critique("CodeReviewAdvisor", true, RiskLevel::Low, "OK"),
            sample_critique("SecurityAdvisor", false, RiskLevel::Low, "Dissent"),
        ];
        let res = resolve_consensus_with_policy(&critiques, &policy);
        assert!(!res.is_approved());
        assert!(res.has_veto());
        assert!(res.veto_reasons.iter().any(|r| r.contains("fell below required policy threshold")));
    }

    #[test]
    fn test_risk_weighted_critical_scale_adjusts_confidence() {
        // Lowering the Critical weight in the policy raises confidence for the
        // same vote set because the critical_scale divides by 8.0.
        let critiques = vec![
            sample_critique("ArchitectureAdvisor", true, RiskLevel::High, "Acceptable"),
            sample_critique("CodeReviewAdvisor", true, RiskLevel::Medium, "Fine"),
        ];

        let res_default = resolve_risk_weighted(&critiques);

        let mut policy = ConsensusPolicy::risk_weighted();
        policy.risk_weights.insert(RiskLevel::Critical, 2.0);
        let res_soft = resolve_consensus_with_policy(&critiques, &policy);

        assert!(res_soft.confidence > res_default.confidence);
    }

    #[test]
    fn test_security_veto_flags_high_risk_even_when_approved() {
        // An approving security advisor that still assesses High risk vetoes.
        let critiques = vec![
            sample_critique("SecurityAdvisor", true, RiskLevel::High, "Runs as root: not advised"),
            sample_critique("ArchitectureAdvisor", true, RiskLevel::Low, "OK"),
            sample_critique("CodeReviewAdvisor", true, RiskLevel::Low, "OK"),
        ];
        let res = resolve_security_veto(&critiques);
        assert!(!res.is_approved());
        assert!(res.has_veto());
        assert!(res.votes[0].vetoed);
        assert_eq!(res.votes[0].score_contribution, 1.0);
    }

    #[test]
    fn test_supermajority_unanimous_and_thresholds() {
        // 3/3 approvals pass any threshold <= 1.0.
        let critiques = vec![
            sample_critique("ArchitectureAdvisor", true, RiskLevel::Low, "OK"),
            sample_critique("CodeReviewAdvisor", true, RiskLevel::Low, "OK"),
            sample_critique("SecurityAdvisor", true, RiskLevel::Low, "OK"),
        ];
        let policy = ConsensusPolicy::supermajority(1.0);
        let res = resolve_consensus_with_policy(&critiques, &policy);
        assert!(res.is_approved());
        assert!((res.confidence - 1.0).abs() < 1e-9);

        // Thresholds are clamped: >1.0 is clamped to 1.0, negative to 0.0.
        let policy_clamped = ConsensusPolicy::supermajority(1.5);
        match policy_clamped.strategy {
            ConsensusStrategy::Supermajority(t) => assert!((t - 1.0).abs() < 1e-9),
            other => panic!("unexpected strategy: {}", other),
        }
    }

    #[test]
    fn test_engine_strategy_override_and_policy_accessors() {
        let mut engine = ConsensusEngine::with_strategy(ConsensusStrategy::Majority);
        assert_eq!(*engine.policy(), ConsensusPolicy::majority());

        let critiques = vec![
            sample_critique("ArchitectureAdvisor", true, RiskLevel::Low, "OK"),
            sample_critique("CodeReviewAdvisor", false, RiskLevel::Low, "Dissent"),
            sample_critique("SecurityAdvisor", true, RiskLevel::Low, "OK"),
        ];

        // Majority (2/3) approves; Unanimous override rejects.
        assert!(engine.resolve(&critiques).is_approved());
        assert!(!engine.resolve_unanimous(&critiques).is_approved());

        // Supermajority override with 0.75 fails at 2/3; 0.60 passes.
        assert!(!engine.resolve_supermajority(&critiques, 0.75).is_approved());
        assert!(engine.resolve_supermajority(&critiques, 0.60).is_approved());

        // Security-veto override passes without a security dissent.
        assert!(engine.resolve_security_veto(&critiques).is_approved());

        // set_policy updates the engine state.
        engine.set_policy(ConsensusPolicy::unanimous());
        assert!(!engine.resolve(&critiques).is_approved());
    }

    #[test]
    fn test_display_and_veto_helpers() {
        assert_eq!(ConsensusStrategy::Majority.to_string(), "Majority (>50%)");
        assert_eq!(ConsensusStrategy::Unanimous.to_string(), "Unanimous (100%)");
        assert_eq!(ConsensusStrategy::RiskWeighted.to_string(), "Risk-Weighted");
        assert_eq!(ConsensusStrategy::SecurityVeto.to_string(), "Security Veto");
        assert_eq!(
            ConsensusStrategy::Supermajority(0.75).to_string(),
            "Supermajority (75.0%)"
        );
        assert_eq!(ConsensusStrategy::default(), ConsensusStrategy::RiskWeighted);
    }

    #[test]
    fn test_veto_badge_rejected_state() {
        let critiques = vec![
            sample_critique("SecurityAdvisor", false, RiskLevel::Critical, "Destructive"),
            sample_critique("ArchitectureAdvisor", true, RiskLevel::Low, "OK"),
        ];
        let res = resolve_majority(&critiques);
        let badge = res.status_badge();
        assert!(badge.contains("✗ REJECTED"));

        let md = res.to_markdown();
        assert!(md.contains("🔴 **REJECTED**"));
        assert!(md.contains("🚫 VETO") || md.contains("❌ Disapprove"));
    }

    #[test]
    fn test_recommendations_deduplicated_across_advisors() {
        let mut c1 = sample_critique("ArchitectureAdvisor", true, RiskLevel::Low, "OK");
        c1.suggestions = vec!["Add tests".to_string(), "  Add tests  ".to_string()];
        let mut c2 = sample_critique("SecurityAdvisor", true, RiskLevel::Low, "OK");
        c2.suggestions = vec!["Add tests".to_string(), "Harden inputs".to_string()];

        let res = resolve_majority(&[c1, c2]);
        assert_eq!(res.recommendations, vec!["Add tests", "Harden inputs"]);
    }
}
