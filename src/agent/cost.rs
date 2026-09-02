//! Token cost tracking, lifetime budgeting, and estimation per provider and model.
//!
//! Supports fine-grained cost estimation covering:
//! - Uncached prompt tokens (input)
//! - Generated completion tokens (output)
//! - Context cache reads / hits (cache read pricing)
//! - Context cache creation / writes (cache write pricing)
//! - Multi-turn session cost tracking with mid-session model switching
//! - Session and lifetime cost accumulation with budget limits
//! - Warning alerts on budget thresholds (50%, 80%, 100%)
//! - Pricing catalogs for OpenAI, Anthropic, DeepSeek, xAI, and OpenRouter
//! - Session cost summaries, budget monitoring, and formatted financial reports

use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::ops::{Add, AddAssign};

use crate::agent::session::{Session, TokenStats};
use crate::config::Config;

/// Detailed breakdown of estimated financial cost in USD.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct CostBreakdown {
    /// Cost for non-cached prompt/input tokens (USD).
    #[serde(default)]
    pub input_cost: f64,

    /// Cost for generated completion/output tokens (USD).
    #[serde(default)]
    pub output_cost: f64,

    /// Cost for cached tokens read from provider context cache (USD).
    #[serde(default)]
    pub cache_read_cost: f64,

    /// Cost for cached tokens written to provider context cache (USD).
    #[serde(default)]
    pub cache_write_cost: f64,

    /// Total accumulated cost (USD).
    #[serde(default)]
    pub total_cost: f64,

    /// Estimated financial savings due to context cache hits (USD).
    #[serde(default)]
    pub cache_savings: f64,
}

impl CostBreakdown {
    /// Creates a new zero-cost breakdown.
    pub const fn zero() -> Self {
        Self {
            input_cost: 0.0,
            output_cost: 0.0,
            cache_read_cost: 0.0,
            cache_write_cost: 0.0,
            total_cost: 0.0,
            cache_savings: 0.0,
        }
    }

    /// Returns `true` if the total cost is effectively zero ($0.00).
    pub fn is_zero(&self) -> bool {
        self.total_cost.abs() < 1e-9
    }

    /// Formats the total cost as a standard USD string (e.g. `"$0.0042"` or `"$1.25"`).
    pub fn format_usd(&self) -> String {
        format_usd(self.total_cost)
    }

    /// Formats the input (prompt) cost as a standard USD string.
    pub fn format_input_usd(&self) -> String {
        format_usd(self.input_cost)
    }

    /// Formats the output (completion) cost as a standard USD string.
    pub fn format_output_usd(&self) -> String {
        format_usd(self.output_cost)
    }

    /// Formats the total cost with 6 decimal places for micro-cent precision.
    pub fn format_precise(&self) -> String {
        format_usd_precise(self.total_cost)
    }

    /// Percentage of costs saved thanks to context caching (0.0 to 100.0%).
    pub fn cache_savings_percentage(&self) -> f64 {
        let baseline = self.total_cost + self.cache_savings;
        if baseline > 1e-9 {
            (self.cache_savings / baseline) * 100.0
        } else {
            0.0
        }
    }

    /// Formats a concise single-line summary of the cost breakdown.
    pub fn format_summary(&self) -> String {
        if self.is_zero() {
            return "$0.00 (free / local model)".to_string();
        }

        let mut summary = format!(
            "Total: {} (In: {}, Out: {})",
            self.format_usd(),
            format_usd(self.input_cost),
            format_usd(self.output_cost)
        );

        if self.cache_read_cost > 0.0 || self.cache_write_cost > 0.0 {
            summary.push_str(&format!(
                ", Cache: [Read: {}, Write: {}]",
                format_usd(self.cache_read_cost),
                format_usd(self.cache_write_cost)
            ));
        }

        if self.cache_savings > 1e-5 {
            summary.push_str(&format!(
                " (Saved: {} / {:.1}%)",
                format_usd(self.cache_savings),
                self.cache_savings_percentage()
            ));
        }

        summary
    }

    /// Formats a multi-line formatted box report.
    pub fn format_table(&self) -> String {
        format!(
            "  Input Cost:       {:>12}\n\
             + Output Cost:      {:>12}\n\
             + Cache Read Cost:  {:>12}\n\
             + Cache Write Cost: {:>12}\n\
             --------------------------------\n\
             = Total Cost:       {:>12}\n\
             (Cache Savings:     {:>12})",
            format_usd_precise(self.input_cost),
            format_usd_precise(self.output_cost),
            format_usd_precise(self.cache_read_cost),
            format_usd_precise(self.cache_write_cost),
            format_usd_precise(self.total_cost),
            format_usd_precise(self.cache_savings),
        )
    }
}

impl Add for CostBreakdown {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self {
            input_cost: self.input_cost + rhs.input_cost,
            output_cost: self.output_cost + rhs.output_cost,
            cache_read_cost: self.cache_read_cost + rhs.cache_read_cost,
            cache_write_cost: self.cache_write_cost + rhs.cache_write_cost,
            total_cost: self.total_cost + rhs.total_cost,
            cache_savings: self.cache_savings + rhs.cache_savings,
        }
    }
}

impl AddAssign for CostBreakdown {
    fn add_assign(&mut self, rhs: Self) {
        *self = *self + rhs;
    }
}

/// Budget monitoring scope: either for the current active session or across lifetime usage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BudgetScope {
    /// Active session budget.
    Session,
    /// Lifetime cumulative budget across all sessions.
    Lifetime,
}

impl std::fmt::Display for BudgetScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BudgetScope::Session => write!(f, "Session"),
            BudgetScope::Lifetime => write!(f, "Lifetime"),
        }
    }
}

/// Predefined or custom financial threshold percentages for budget warning alerts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum BudgetThreshold {
    /// 50% of budget consumed (Advisory threshold).
    FiftyPercent,
    /// 80% of budget consumed (High utilization warning).
    EightyPercent,
    /// 100% of budget consumed (Limit reached or exceeded).
    Exceeded,
    /// Custom threshold percentage (e.g. 90%).
    Custom(u32),
}

impl BudgetThreshold {
    /// Returns the numerical percentage (e.g. 50.0, 80.0, 100.0).
    pub fn percentage(&self) -> f64 {
        match self {
            BudgetThreshold::FiftyPercent => 50.0,
            BudgetThreshold::EightyPercent => 80.0,
            BudgetThreshold::Exceeded => 100.0,
            BudgetThreshold::Custom(pct) => *pct as f64,
        }
    }

    /// Human-readable label for the threshold tier.
    pub fn label(&self) -> String {
        match self {
            BudgetThreshold::FiftyPercent => "50% Warning".to_string(),
            BudgetThreshold::EightyPercent => "80% Alert".to_string(),
            BudgetThreshold::Exceeded => "100% Exceeded".to_string(),
            BudgetThreshold::Custom(pct) => format!("{}% Threshold", pct),
        }
    }
}

/// Warning or critical alert generated when spending crosses a budget threshold.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BudgetAlert {
    /// The threshold reached or exceeded.
    pub threshold: BudgetThreshold,
    /// Scope of the budget alert (Session or Lifetime).
    pub scope: BudgetScope,
    /// Current total spending in USD at the time of the alert.
    pub current_cost: f64,
    /// Configured budget limit in USD.
    pub budget_limit: f64,
    /// Remaining budget in USD (0.0 if exceeded).
    pub remaining: f64,
    /// Percentage of budget consumed.
    pub percentage_used: f64,
    /// Human-readable alert description.
    pub message: String,
}

impl BudgetAlert {
    /// Constructs a new `BudgetAlert`.
    pub fn new(
        threshold: BudgetThreshold,
        scope: BudgetScope,
        current_cost: f64,
        budget_limit: f64,
    ) -> Self {
        let remaining = (budget_limit - current_cost).max(0.0);
        let percentage_used = if budget_limit > 1e-9 {
            (current_cost / budget_limit) * 100.0
        } else {
            100.0
        };

        let message = match threshold {
            BudgetThreshold::FiftyPercent => format!(
                "⚠️ {} Budget Warning (50%): Current spend has reached {} of {} budget limit ({} remaining).",
                scope,
                format_usd(current_cost),
                format_usd(budget_limit),
                format_usd(remaining)
            ),
            BudgetThreshold::EightyPercent => format!(
                "⚠️ {} Budget Alert (80%): Current spend has reached {} of {} budget limit ({} remaining).",
                scope,
                format_usd(current_cost),
                format_usd(budget_limit),
                format_usd(remaining)
            ),
            BudgetThreshold::Exceeded => format!(
                "🚨 {} Budget Limit Exceeded (100%): Current spend of {} has reached or exceeded the {} budget limit.",
                scope,
                format_usd(current_cost),
                format_usd(budget_limit)
            ),
            BudgetThreshold::Custom(pct) => format!(
                "⚠️ {} Budget Threshold ({}%): Current spend has reached {} of {} budget limit ({} remaining).",
                scope,
                pct,
                format_usd(current_cost),
                format_usd(budget_limit),
                format_usd(remaining)
            ),
        };

        Self {
            threshold,
            scope,
            current_cost,
            budget_limit,
            remaining,
            percentage_used,
            message,
        }
    }

    /// Returns `true` if this alert represents a critical 100% budget breach.
    pub fn is_critical(&self) -> bool {
        matches!(self.threshold, BudgetThreshold::Exceeded) || self.percentage_used >= 100.0
    }

    /// Returns `true` if this alert is an advisory warning (below 100%).
    pub fn is_warning(&self) -> bool {
        !self.is_critical()
    }

    /// Formats the alert message with optional ANSI coloring for terminal outputs.
    pub fn format_alert(&self) -> String {
        self.message.clone()
    }
}

/// Token pricing specification for a specific model (all rates in USD per 1,000,000 tokens).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelPricing {
    /// Canonical provider name (e.g. `"anthropic"`, `"openai"`, `"deepseek"`, `"xai"`, `"openrouter"`).
    pub provider: String,

    /// Canonical model name or identifier pattern (e.g. `"claude-3-7-sonnet-20250219"`).
    pub model: String,

    /// Price in USD per 1M non-cached prompt/input tokens.
    pub input_per_million: f64,

    /// Price in USD per 1M completion/output tokens.
    pub output_per_million: f64,

    /// Price in USD per 1M context cache read / hit tokens.
    pub cache_read_per_million: f64,

    /// Price in USD per 1M context cache creation / write tokens.
    pub cache_write_per_million: f64,
}

impl ModelPricing {
    /// Creates a complete pricing specification.
    pub fn new(
        provider: impl Into<String>,
        model: impl Into<String>,
        input_per_million: f64,
        output_per_million: f64,
        cache_read_per_million: f64,
        cache_write_per_million: f64,
    ) -> Self {
        Self {
            provider: provider.into(),
            model: model.into(),
            input_per_million,
            output_per_million,
            cache_read_per_million,
            cache_write_per_million,
        }
    }

    /// Creates a free model specification ($0.00 for all metrics, e.g. for Ollama / local inference).
    pub fn free(provider: impl Into<String>, model: impl Into<String>) -> Self {
        Self::new(provider, model, 0.0, 0.0, 0.0, 0.0)
    }

    /// Creates a simple model specification with no cache discounts.
    pub fn simple(
        provider: impl Into<String>,
        model: impl Into<String>,
        input_per_million: f64,
        output_per_million: f64,
    ) -> Self {
        Self::new(
            provider,
            model,
            input_per_million,
            output_per_million,
            input_per_million,
            input_per_million,
        )
    }

    /// Standard Anthropic pricing tier:
    /// - Cache Read = 10% of base input price (90% discount)
    /// - Cache Write = 125% of base input price (25% surcharge)
    pub fn anthropic_tier(
        model: impl Into<String>,
        input_per_million: f64,
        output_per_million: f64,
    ) -> Self {
        let in_rate = input_per_million;
        Self::new(
            "anthropic",
            model,
            in_rate,
            output_per_million,
            in_rate * 0.10,
            in_rate * 1.25,
        )
    }

    /// Standard OpenAI pricing tier:
    /// - Cache Read (hit) = 50% of base input price (50% discount)
    /// - Cache Write = base input price (no write surcharge)
    pub fn openai_tier(
        model: impl Into<String>,
        input_per_million: f64,
        output_per_million: f64,
    ) -> Self {
        let in_rate = input_per_million;
        Self::new(
            "openai",
            model,
            in_rate,
            output_per_million,
            in_rate * 0.50,
            in_rate,
        )
    }

    /// Standard DeepSeek pricing tier:
    /// - Cache Read (hit) is heavily discounted
    /// - Cache Write / Miss is base input price
    pub fn deepseek_tier(
        model: impl Into<String>,
        input_miss_per_million: f64,
        input_hit_per_million: f64,
        output_per_million: f64,
    ) -> Self {
        Self::new(
            "deepseek",
            model,
            input_miss_per_million,
            output_per_million,
            input_hit_per_million,
            input_miss_per_million,
        )
    }

    /// Standard xAI pricing tier:
    /// - Cache Read = 50% of base input price
    /// - Cache Write = base input price
    pub fn xai_tier(
        model: impl Into<String>,
        input_per_million: f64,
        output_per_million: f64,
    ) -> Self {
        let in_rate = input_per_million;
        Self::new(
            "xai",
            model,
            in_rate,
            output_per_million,
            in_rate * 0.50,
            in_rate,
        )
    }

    /// Standard Google / Gemini pricing tier:
    /// - Cache Read = 25% of base input price
    /// - Cache Write = base input price
    pub fn google_tier(
        model: impl Into<String>,
        input_per_million: f64,
        output_per_million: f64,
    ) -> Self {
        let in_rate = input_per_million;
        Self::new(
            "google",
            model,
            in_rate,
            output_per_million,
            in_rate * 0.25,
            in_rate,
        )
    }

    /// Computes the exact monetary cost breakdown for given token counts.
    ///
    /// If `cache_read_tokens` are present:
    /// Non-cached prompt tokens are computed as `prompt_tokens.saturating_sub(cache_read_tokens)`.
    /// Savings are computed against the full base input price.
    pub fn calculate(
        &self,
        prompt_tokens: u64,
        completion_tokens: u64,
        cache_read_tokens: u64,
        cache_write_tokens: u64,
    ) -> CostBreakdown {
        let uncached_prompt = prompt_tokens.saturating_sub(cache_read_tokens);

        let input_cost = (uncached_prompt as f64 / 1_000_000.0) * self.input_per_million;
        let output_cost = (completion_tokens as f64 / 1_000_000.0) * self.output_per_million;
        let cache_read_cost = (cache_read_tokens as f64 / 1_000_000.0) * self.cache_read_per_million;
        let cache_write_cost =
            (cache_write_tokens as f64 / 1_000_000.0) * self.cache_write_per_million;

        let total_cost = input_cost + output_cost + cache_read_cost + cache_write_cost;

        let cache_savings = if cache_read_tokens > 0 {
            let baseline_cost = (cache_read_tokens as f64 / 1_000_000.0) * self.input_per_million;
            (baseline_cost - cache_read_cost).max(0.0)
        } else {
            0.0
        };

        CostBreakdown {
            input_cost,
            output_cost,
            cache_read_cost,
            cache_write_cost,
            total_cost,
            cache_savings,
        }
    }

    /// Computes cost for a single turn with prompt and completion counts only.
    pub fn calculate_turn(&self, prompt_tokens: u64, completion_tokens: u64) -> CostBreakdown {
        self.calculate(prompt_tokens, completion_tokens, 0, 0)
    }

    /// Computes cost from an accumulated `TokenStats` structure.
    pub fn calculate_from_stats(&self, stats: &TokenStats) -> CostBreakdown {
        self.calculate(
            stats.prompt_tokens,
            stats.completion_tokens,
            stats.cache_read_tokens,
            stats.cache_write_tokens,
        )
    }
}

/// Detailed historical record of a single conversational turn's cost and usage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CostTurnRecord {
    /// ISO 8601 timestamp when the turn was executed.
    pub timestamp: String,
    /// 1-based turn index within the session.
    pub turn_index: usize,
    /// Provider utilized for this turn.
    pub provider: String,
    /// Model utilized for this turn.
    pub model: String,
    /// Prompt tokens consumed.
    pub prompt_tokens: u64,
    /// Completion tokens consumed.
    pub completion_tokens: u64,
    /// Context cache read tokens.
    pub cache_read_tokens: u64,
    /// Context cache write tokens.
    pub cache_write_tokens: u64,
    /// Cost breakdown for this turn.
    pub breakdown: CostBreakdown,
}

/// Lifetime cumulative cost and token statistics preserved across sessions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LifetimeCostStats {
    /// Number of completed sessions recorded in lifetime totals.
    pub session_count: usize,
    /// Number of turns completed across all sessions.
    pub total_turns: usize,
    /// Total tokens consumed across all sessions.
    pub total_tokens: u64,
    /// Cumulative cost breakdown across all sessions.
    pub total_breakdown: CostBreakdown,
    /// Cumulative cost breakdown per model.
    pub model_breakdown: HashMap<String, CostBreakdown>,
    /// Cumulative cost breakdown per provider.
    pub provider_breakdown: HashMap<String, CostBreakdown>,
    /// Optional lifetime budget limit in USD.
    pub budget_limit_usd: Option<f64>,
}

impl LifetimeCostStats {
    /// Creates a new empty lifetime cost accumulator.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a lifetime cost accumulator with a pre-configured budget cap.
    pub fn with_budget(budget_limit_usd: f64) -> Self {
        Self {
            budget_limit_usd: Some(budget_limit_usd.max(0.0)),
            ..Default::default()
        }
    }

    /// Sets the lifetime budget limit in USD.
    pub fn set_budget_limit(&mut self, limit_usd: f64) {
        self.budget_limit_usd = Some(limit_usd.max(0.0));
    }

    /// Returns the lifetime budget limit, if set.
    pub fn budget_limit(&self) -> Option<f64> {
        self.budget_limit_usd
    }

    /// Returns remaining lifetime budget in USD, or `None` if no budget was set.
    pub fn budget_remaining(&self) -> Option<f64> {
        self.budget_limit_usd
            .map(|limit| (limit - self.total_breakdown.total_cost).max(0.0))
    }

    /// Percentage of lifetime budget consumed (0.0 to 100.0+%), or `None` if unset.
    pub fn budget_percentage_used(&self) -> Option<f64> {
        self.budget_limit_usd.map(|limit| {
            if limit > 1e-9 {
                (self.total_breakdown.total_cost / limit) * 100.0
            } else {
                100.0
            }
        })
    }

    /// Returns `true` if current accumulated lifetime expenditure exceeds the budget limit.
    pub fn is_budget_exceeded(&self) -> bool {
        match self.budget_limit_usd {
            Some(limit) => self.total_breakdown.total_cost > limit,
            None => false,
        }
    }

    /// Accumulates a single turn into lifetime statistics.
    pub fn record_turn(
        &mut self,
        provider: &str,
        model: &str,
        prompt_tokens: u64,
        completion_tokens: u64,
        breakdown: CostBreakdown,
    ) {
        self.total_turns += 1;
        self.total_tokens = self
            .total_tokens
            .saturating_add(prompt_tokens)
            .saturating_add(completion_tokens);
        self.total_breakdown += breakdown;

        *self
            .model_breakdown
            .entry(model.to_string())
            .or_insert_with(CostBreakdown::zero) += breakdown;

        *self
            .provider_breakdown
            .entry(provider.to_string())
            .or_insert_with(CostBreakdown::zero) += breakdown;
    }

    /// Incorporates a completed session into lifetime statistics.
    pub fn record_session(
        &mut self,
        session_breakdown: &CostBreakdown,
        session_turns: usize,
        session_tokens: u64,
        session_model_breakdown: &HashMap<String, CostBreakdown>,
        session_provider_breakdown: &HashMap<String, CostBreakdown>,
    ) {
        self.session_count += 1;
        self.total_turns += session_turns;
        self.total_tokens = self.total_tokens.saturating_add(session_tokens);
        self.total_breakdown += *session_breakdown;

        for (model, cost) in session_model_breakdown {
            *self
                .model_breakdown
                .entry(model.clone())
                .or_insert_with(CostBreakdown::zero) += *cost;
        }

        for (prov, cost) in session_provider_breakdown {
            *self
                .provider_breakdown
                .entry(prov.clone())
                .or_insert_with(CostBreakdown::zero) += *cost;
        }
    }

    /// Resets all lifetime statistics back to zero.
    pub fn reset(&mut self) {
        self.session_count = 0;
        self.total_turns = 0;
        self.total_tokens = 0;
        self.total_breakdown = CostBreakdown::zero();
        self.model_breakdown.clear();
        self.provider_breakdown.clear();
    }

    /// Formats a concise one-line lifetime cost summary.
    pub fn format_summary(&self) -> String {
        format!(
            "Lifetime Spent: {} across {} session{} ({} turns, {} tokens)",
            self.total_breakdown.format_usd(),
            self.session_count,
            if self.session_count == 1 { "" } else { "s" },
            self.total_turns,
            self.total_tokens
        )
    }

    /// Generates a detailed multi-section lifetime cost breakdown report.
    pub fn format_detailed_report(&self) -> String {
        let mut report = String::new();
        report.push_str("=== Lifetime Cost & Token Usage Report ===\n");
        report.push_str(&format!("Grand Total Lifetime Cost: {}\n", self.total_breakdown.format_usd()));
        report.push_str(&format!("Total Sessions Recorded:  {}\n", self.session_count));
        report.push_str(&format!("Total Turns Completed:    {}\n", self.total_turns));
        report.push_str(&format!("Total Tokens Consumed:    {}\n", self.total_tokens));

        if self.total_breakdown.cache_savings > 1e-5 {
            report.push_str(&format!(
                "Lifetime Cache Savings:   {} ({:.1}%)\n",
                format_usd(self.total_breakdown.cache_savings),
                self.total_breakdown.cache_savings_percentage()
            ));
        }

        if let Some(budget) = self.budget_limit_usd {
            let remaining = self.budget_remaining().unwrap_or(0.0);
            let pct = self.budget_percentage_used().unwrap_or(0.0);
            report.push_str(&format!(
                "Lifetime Budget Cap:      {} (Remaining: {}, {:.1}% used)\n",
                format_usd(budget),
                format_usd(remaining),
                pct
            ));
        }

        report.push_str("\n--- Lifetime Breakdown by Model ---\n");
        let mut sorted_models: Vec<_> = self.model_breakdown.iter().collect();
        sorted_models.sort_by(|a, b| b.1.total_cost.partial_cmp(&a.1.total_cost).unwrap_or(std::cmp::Ordering::Equal));
        for (model, cost) in sorted_models {
            report.push_str(&format!(
                "  • {:<30} {:>10} (In: {}, Out: {})\n",
                model,
                cost.format_usd(),
                format_usd(cost.input_cost),
                format_usd(cost.output_cost)
            ));
        }

        report.push_str("\n--- Lifetime Breakdown by Provider ---\n");
        let mut sorted_provs: Vec<_> = self.provider_breakdown.iter().collect();
        sorted_provs.sort_by(|a, b| b.1.total_cost.partial_cmp(&a.1.total_cost).unwrap_or(std::cmp::Ordering::Equal));
        for (provider, cost) in sorted_provs {
            report.push_str(&format!(
                "  • {:<15} {:>10}\n",
                provider,
                cost.format_usd()
            ));
        }

        report
    }
}

/// Global registry of model pricing with built-in database and custom overrides.
#[derive(Debug, Clone)]
pub struct ModelPricingRegistry {
    custom_pricing: HashMap<String, ModelPricing>,
}

impl Default for ModelPricingRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ModelPricingRegistry {
    /// Creates a new pricing registry.
    pub fn new() -> Self {
        Self {
            custom_pricing: HashMap::new(),
        }
    }

    /// Registers or overrides pricing for a specific model key (`"provider:model"` or `"model"`).
    pub fn register(&mut self, pricing: ModelPricing) {
        let key = format!("{}:{}", pricing.provider.to_lowercase(), pricing.model.to_lowercase());
        self.custom_pricing.insert(key, pricing);
    }

    /// Resolves pricing for a given provider and model identifier.
    pub fn get(&self, provider_hint: &str, model_input: &str) -> ModelPricing {
        let (resolved_provider, resolved_model) =
            Config::resolve_model(model_input, Some(provider_hint));

        let norm_prov = resolved_provider.trim().to_lowercase();
        let norm_model = resolved_model.trim().to_lowercase();

        // 1. Check custom overrides first
        let exact_key = format!("{}:{}", norm_prov, norm_model);
        if let Some(custom) = self.custom_pricing.get(&exact_key) {
            return custom.clone();
        }

        // 2. Ollama is always free (local inference)
        if norm_prov == "ollama" {
            return ModelPricing::free("ollama", resolved_model);
        }

        // 3. Match against built-in pricing catalog
        Self::lookup_builtin(&norm_prov, &norm_model)
            .unwrap_or_else(|| Self::fallback_pricing(&norm_prov, &resolved_model))
    }

    /// Look up built-in pricing catalog with model normalization and alias matching.
    fn lookup_builtin(provider: &str, model: &str) -> Option<ModelPricing> {
        match provider {
            "anthropic" => Self::lookup_anthropic(model),
            "openai" => Self::lookup_openai(model),
            "deepseek" => Self::lookup_deepseek(model),
            "xai" | "x-ai" => Self::lookup_xai(model),
            "openrouter" => Self::lookup_openrouter(model),
            "google" => Self::lookup_google(model),
            _ => None,
        }
    }

    fn lookup_anthropic(model: &str) -> Option<ModelPricing> {
        let m = model.trim().to_lowercase();

        // Claude 3.7 Sonnet ($3.00 / $15.00 / $0.30 / $3.75)
        if m.contains("3-7-sonnet") || m.contains("3.7-sonnet") || m == "sonnet-3.7" || m == "3.7" {
            return Some(ModelPricing::anthropic_tier("claude-3-7-sonnet-20250219", 3.00, 15.00));
        }

        // Claude 3.5 Sonnet ($3.00 / $15.00 / $0.30 / $3.75)
        if m.contains("3-5-sonnet") || m.contains("3.5-sonnet") || m.contains("sonnet") {
            return Some(ModelPricing::anthropic_tier("claude-3-5-sonnet-20241022", 3.00, 15.00));
        }

        // Claude 3.5 Haiku ($0.80 / $4.00 / $0.08 / $1.00)
        if m.contains("3-5-haiku") || m.contains("3.5-haiku") || m.contains("haiku-3.5") {
            return Some(ModelPricing::anthropic_tier("claude-3-5-haiku-20241022", 0.80, 4.00));
        }

        // Claude 3 Haiku ($0.25 / $1.25 / $0.03 / $0.30)
        if m.contains("haiku") {
            return Some(ModelPricing::anthropic_tier("claude-3-haiku-20240307", 0.25, 1.25));
        }

        // Claude 3 Opus ($15.00 / $75.00 / $1.50 / $18.75)
        if m.contains("opus") {
            return Some(ModelPricing::anthropic_tier("claude-3-opus-20240229", 15.00, 75.00));
        }

        // Default Anthropic tier (Sonnet pricing)
        Some(ModelPricing::anthropic_tier(model, 3.00, 15.00))
    }

    fn lookup_openai(model: &str) -> Option<ModelPricing> {
        let m = model.trim().to_lowercase();

        // GPT-4.5 Preview ($75.00 / $150.00 / $37.50 / $75.00)
        if m.contains("4.5") || m.contains("4-5") {
            return Some(ModelPricing::openai_tier("gpt-4.5-preview", 75.00, 150.00));
        }

        // GPT-4o Mini ($0.15 / $0.60 / $0.075 / $0.15)
        if m.contains("4o-mini") || m.contains("gpt-4o-mini") {
            return Some(ModelPricing::openai_tier("gpt-4o-mini", 0.15, 0.60));
        }

        // GPT-4o ($2.50 / $10.00 / $1.25 / $2.50)
        if m.contains("4o") || m.contains("gpt-4o") {
            return Some(ModelPricing::openai_tier("gpt-4o", 2.50, 10.00));
        }

        // o3-mini ($1.10 / $4.40 / $0.55 / $1.10)
        if m.contains("o3-mini") || m == "o3-mini" {
            return Some(ModelPricing::openai_tier("o3-mini", 1.10, 4.40));
        }

        // o3 ($15.00 / $60.00 / $7.50 / $15.00)
        if m == "o3" || m.starts_with("o3-") {
            return Some(ModelPricing::openai_tier("o3", 15.00, 60.00));
        }

        // o1 ($15.00 / $60.00 / $7.50 / $15.00)
        if m == "o1" || m.starts_with("o1-2024") || m == "o1-preview" {
            return Some(ModelPricing::openai_tier("o1", 15.00, 60.00));
        }

        // o1-mini ($1.10 / $4.40 / $0.55 / $1.10)
        if m.contains("o1-mini") {
            return Some(ModelPricing::openai_tier("o1-mini", 1.10, 4.40));
        }

        // GPT-4 Turbo ($10.00 / $30.00 / $5.00 / $10.00)
        if m.contains("turbo") && m.contains("4") {
            return Some(ModelPricing::openai_tier("gpt-4-turbo", 10.00, 30.00));
        }

        // Legacy GPT-4 32k ($60.00 / $120.00)
        if m.contains("32k") && m.contains("gpt-4") {
            return Some(ModelPricing::simple("openai", "gpt-4-32k", 60.00, 120.00));
        }

        // Legacy GPT-4 ($30.00 / $60.00)
        if m == "gpt-4" || m.starts_with("gpt-4-0") {
            return Some(ModelPricing::simple("openai", "gpt-4", 30.00, 60.00));
        }

        // GPT-3.5 Turbo ($0.50 / $1.50 / $0.25 / $0.50)
        if m.contains("3.5-turbo") || m.contains("3.5") {
            return Some(ModelPricing::openai_tier("gpt-3.5-turbo", 0.50, 1.50));
        }

        // Embeddings
        if m.contains("text-embedding-3-small") {
            return Some(ModelPricing::simple("openai", "text-embedding-3-small", 0.02, 0.00));
        }
        if m.contains("text-embedding-3-large") {
            return Some(ModelPricing::simple("openai", "text-embedding-3-large", 0.13, 0.00));
        }
        if m.contains("text-embedding-ada-002") {
            return Some(ModelPricing::simple("openai", "text-embedding-ada-002", 0.10, 0.00));
        }

        // Default OpenAI fallback
        Some(ModelPricing::openai_tier(model, 2.50, 10.00))
    }

    fn lookup_deepseek(model: &str) -> Option<ModelPricing> {
        let m = model.trim().to_lowercase();

        // DeepSeek Reasoner / R1 ($0.55 input miss / $0.14 input hit / $2.19 output)
        if m.contains("reasoner") || m.contains("r1") {
            return Some(ModelPricing::deepseek_tier("deepseek-reasoner", 0.55, 0.14, 2.19));
        }

        // DeepSeek Chat / V3 ($0.14 input miss / $0.014 input hit / $0.28 output)
        if m.contains("chat") || m.contains("v3") || m == "deepseek" {
            return Some(ModelPricing::deepseek_tier("deepseek-chat", 0.14, 0.014, 0.28));
        }

        // DeepSeek Coder ($0.14 / $0.014 / $0.28)
        if m.contains("coder") {
            return Some(ModelPricing::deepseek_tier("deepseek-coder", 0.14, 0.014, 0.28));
        }

        // Default DeepSeek tier
        Some(ModelPricing::deepseek_tier(model, 0.14, 0.014, 0.28))
    }

    fn lookup_xai(model: &str) -> Option<ModelPricing> {
        let m = model.trim().to_lowercase();

        // Grok-3 ($3.00 / $15.00 / $1.50 / $3.00)
        if m.contains("grok-3-mini") {
            return Some(ModelPricing::xai_tier("grok-3-mini", 0.30, 1.50));
        }
        if m.contains("grok-3") {
            return Some(ModelPricing::xai_tier("grok-3", 3.00, 15.00));
        }

        // Grok-2 ($2.00 / $10.00 / $1.00 / $2.00)
        if m.contains("grok-2-mini") {
            return Some(ModelPricing::xai_tier("grok-2-mini", 0.20, 1.00));
        }
        if m.contains("grok-2") || m == "grok" || m == "grok-2-latest" {
            return Some(ModelPricing::xai_tier("grok-2-latest", 2.00, 10.00));
        }

        // Grok-Beta ($5.00 / $15.00 / $5.00 / $5.00)
        if m.contains("beta") {
            return Some(ModelPricing::new("xai", "grok-beta", 5.00, 15.00, 5.00, 5.00));
        }

        Some(ModelPricing::xai_tier(model, 2.00, 10.00))
    }

    fn lookup_google(model: &str) -> Option<ModelPricing> {
        let m = model.trim().to_lowercase();

        // Gemini 2.0 Flash ($0.10 / $0.40 / $0.025 / $0.10)
        if m.contains("gemini-2.0-flash") || m.contains("2.0-flash") {
            return Some(ModelPricing::google_tier("gemini-2.0-flash", 0.10, 0.40));
        }

        // Gemini 2.5 Pro / 2.0 Pro ($1.25 / $5.00 / $0.3125 / $1.25)
        if m.contains("gemini-2.5-pro") || m.contains("gemini-2.0-pro") || m.contains("2.5-pro") {
            return Some(ModelPricing::google_tier("gemini-2.5-pro", 1.25, 5.00));
        }

        // Gemini 1.5 Pro ($1.25 / $5.00 / $0.3125 / $1.25)
        if m.contains("gemini-1.5-pro") || m.contains("1.5-pro") {
            return Some(ModelPricing::google_tier("gemini-1.5-pro", 1.25, 5.00));
        }

        // Gemini 1.5 Flash ($0.075 / $0.30 / $0.01875 / $0.075)
        if m.contains("gemini-1.5-flash") || m.contains("1.5-flash") {
            return Some(ModelPricing::google_tier("gemini-1.5-flash", 0.075, 0.30));
        }

        Some(ModelPricing::google_tier(model, 1.25, 5.00))
    }

    fn lookup_openrouter(model: &str) -> Option<ModelPricing> {
        let m = model.trim().to_lowercase();

        // 1. If OpenRouter model has a sub-provider prefix (e.g. "anthropic/claude-3.5-sonnet")
        if let Some((sub_prov, sub_model)) = m.split_once('/') {
            let norm_sub_prov = match sub_prov {
                "x-ai" => "xai",
                other => other,
            };

            if let Some(pricing) = Self::lookup_builtin(norm_sub_prov, sub_model) {
                return Some(ModelPricing {
                    provider: "openrouter".to_string(),
                    model: model.to_string(),
                    ..pricing
                });
            }

            // Check specific OpenRouter open-weights catalog
            if sub_prov == "meta-llama" || sub_prov == "llama" {
                if sub_model.contains("70b") {
                    return Some(ModelPricing::simple("openrouter", model, 0.12, 0.30));
                }
                if sub_model.contains("405b") {
                    return Some(ModelPricing::simple("openrouter", model, 2.00, 2.00));
                }
                if sub_model.contains("8b") {
                    return Some(ModelPricing::simple("openrouter", model, 0.03, 0.05));
                }
                return Some(ModelPricing::simple("openrouter", model, 0.12, 0.30));
            }

            if sub_prov == "mistralai" || sub_prov == "mistral" {
                if sub_model.contains("large") {
                    return Some(ModelPricing::simple("openrouter", model, 2.00, 6.00));
                }
                if sub_model.contains("small") {
                    return Some(ModelPricing::simple("openrouter", model, 0.10, 0.30));
                }
                if sub_model.contains("codestral") {
                    return Some(ModelPricing::simple("openrouter", model, 0.30, 0.90));
                }
                return Some(ModelPricing::simple("openrouter", model, 0.50, 1.50));
            }

            if sub_prov == "qwen" {
                if sub_model.contains("coder-32b") || sub_model.contains("2.5-coder-32b") {
                    return Some(ModelPricing::simple("openrouter", model, 0.07, 0.16));
                }
                if sub_model.contains("72b") {
                    return Some(ModelPricing::simple("openrouter", model, 0.12, 0.35));
                }
                if sub_model.contains("qwq") {
                    return Some(ModelPricing::simple("openrouter", model, 0.12, 0.18));
                }
                return Some(ModelPricing::simple("openrouter", model, 0.10, 0.25));
            }
        }

        // 2. Direct name matching without prefix
        if m.contains("deepseek") {
            return Self::lookup_deepseek(&m).map(|p| ModelPricing {
                provider: "openrouter".to_string(),
                model: model.to_string(),
                ..p
            });
        }
        if m.contains("claude") {
            return Self::lookup_anthropic(&m).map(|p| ModelPricing {
                provider: "openrouter".to_string(),
                model: model.to_string(),
                ..p
            });
        }
        if m.contains("gpt") || m.contains("o1") || m.contains("o3") {
            return Self::lookup_openai(&m).map(|p| ModelPricing {
                provider: "openrouter".to_string(),
                model: model.to_string(),
                ..p
            });
        }
        if m.contains("grok") {
            return Self::lookup_xai(&m).map(|p| ModelPricing {
                provider: "openrouter".to_string(),
                model: model.to_string(),
                ..p
            });
        }
        if m.contains("gemini") {
            return Self::lookup_google(&m).map(|p| ModelPricing {
                provider: "openrouter".to_string(),
                model: model.to_string(),
                ..p
            });
        }

        // Fallback openrouter pricing ($1.00 in / $3.00 out)
        Some(ModelPricing::simple("openrouter", model, 1.00, 3.00))
    }

    /// Default sensible pricing fallback when model is completely unrecognized.
    fn fallback_pricing(provider: &str, model: &str) -> ModelPricing {
        ModelPricing::new(provider, model, 1.00, 3.00, 0.50, 1.00)
    }
}

/// Comprehensive turn-by-turn and multi-model session & lifetime cost tracker.
#[derive(Debug, Clone)]
pub struct CostTracker {
    /// History of completed turns and their individual costs for the current session.
    turns: Vec<CostTurnRecord>,

    /// Aggregate cost breakdown broken down per canonical model for the current session.
    model_breakdown: HashMap<String, CostBreakdown>,

    /// Aggregate cost breakdown broken down per provider for the current session.
    provider_breakdown: HashMap<String, CostBreakdown>,

    /// Running grand total cost breakdown for the current session.
    total_breakdown: CostBreakdown,

    /// Total tokens consumed across all turns in the current session.
    total_tokens: u64,

    /// Pricing registry for rate lookups.
    registry: ModelPricingRegistry,

    /// Optional session budget cap in USD.
    budget_limit_usd: Option<f64>,

    /// Lifetime accumulation stats across sessions.
    lifetime_stats: LifetimeCostStats,

    /// Set of session budget thresholds that have already triggered an alert in this session.
    triggered_session_alerts: HashSet<BudgetThreshold>,

    /// Set of lifetime budget thresholds that have already triggered an alert.
    triggered_lifetime_alerts: HashSet<BudgetThreshold>,

    /// Alerts generated in the most recent turn / check.
    recent_alerts: Vec<BudgetAlert>,
}

impl Default for CostTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl CostTracker {
    /// Creates a new empty cost tracker.
    pub fn new() -> Self {
        Self {
            turns: Vec::new(),
            model_breakdown: HashMap::new(),
            provider_breakdown: HashMap::new(),
            total_breakdown: CostBreakdown::zero(),
            total_tokens: 0,
            registry: ModelPricingRegistry::new(),
            budget_limit_usd: None,
            lifetime_stats: LifetimeCostStats::new(),
            triggered_session_alerts: HashSet::new(),
            triggered_lifetime_alerts: HashSet::new(),
            recent_alerts: Vec::new(),
        }
    }

    /// Creates a cost tracker with a custom pricing registry.
    pub fn with_registry(registry: ModelPricingRegistry) -> Self {
        Self {
            turns: Vec::new(),
            model_breakdown: HashMap::new(),
            provider_breakdown: HashMap::new(),
            total_breakdown: CostBreakdown::zero(),
            total_tokens: 0,
            registry,
            budget_limit_usd: None,
            lifetime_stats: LifetimeCostStats::new(),
            triggered_session_alerts: HashSet::new(),
            triggered_lifetime_alerts: HashSet::new(),
            recent_alerts: Vec::new(),
        }
    }

    /// Creates a cost tracker with pre-configured session and lifetime budget limits.
    pub fn with_budgets(session_budget_usd: Option<f64>, lifetime_budget_usd: Option<f64>) -> Self {
        let mut tracker = Self::new();
        if let Some(sb) = session_budget_usd {
            tracker.set_session_budget(sb);
        }
        if let Some(lb) = lifetime_budget_usd {
            tracker.set_lifetime_budget(lb);
        }
        tracker
    }

    /// Sets an advisory or strict session budget cap in USD.
    pub fn set_budget_limit(&mut self, limit_usd: f64) {
        self.set_session_budget(limit_usd);
    }

    /// Sets the session budget limit in USD and resets triggered session alerts.
    pub fn set_session_budget(&mut self, limit_usd: f64) {
        self.budget_limit_usd = Some(limit_usd.max(0.0));
        self.triggered_session_alerts.clear();
        self.check_budget_alerts();
    }

    /// Sets the lifetime budget limit in USD and resets triggered lifetime alerts.
    pub fn set_lifetime_budget(&mut self, limit_usd: f64) {
        self.lifetime_stats.set_budget_limit(limit_usd);
        self.triggered_lifetime_alerts.clear();
        self.check_budget_alerts();
    }

    /// Returns the currently configured session budget cap, if any.
    pub fn budget_limit(&self) -> Option<f64> {
        self.budget_limit_usd
    }

    /// Returns the session budget limit, if any.
    pub fn session_budget_limit(&self) -> Option<f64> {
        self.budget_limit_usd
    }

    /// Returns the lifetime budget limit, if any.
    pub fn lifetime_budget_limit(&self) -> Option<f64> {
        self.lifetime_stats.budget_limit()
    }

    /// Returns remaining session budget in USD, or `None` if no session budget was set.
    pub fn budget_remaining(&self) -> Option<f64> {
        self.budget_limit_usd
            .map(|limit| (limit - self.total_breakdown.total_cost).max(0.0))
    }

    /// Returns remaining lifetime budget in USD, or `None` if no lifetime budget was set.
    pub fn lifetime_budget_remaining(&self) -> Option<f64> {
        self.lifetime_stats.budget_remaining()
    }

    /// Returns the percentage of session budget consumed (e.g. 52.4%), or `None` if unset.
    pub fn session_budget_percentage(&self) -> Option<f64> {
        self.budget_limit_usd.map(|limit| {
            if limit > 1e-9 {
                (self.total_breakdown.total_cost / limit) * 100.0
            } else {
                100.0
            }
        })
    }

    /// Returns the percentage of lifetime budget consumed, or `None` if unset.
    pub fn lifetime_budget_percentage(&self) -> Option<f64> {
        self.lifetime_stats.budget_percentage_used()
    }

    /// Returns `true` if current accumulated session expenditure exceeds the session budget limit.
    pub fn is_budget_exceeded(&self) -> bool {
        match self.budget_limit_usd {
            Some(limit) => self.total_breakdown.total_cost > limit,
            None => false,
        }
    }

    /// Returns `true` if current accumulated lifetime expenditure exceeds the lifetime budget limit.
    pub fn is_lifetime_budget_exceeded(&self) -> bool {
        self.lifetime_stats.is_budget_exceeded()
    }

    /// Reference to the cumulative lifetime cost statistics.
    pub fn lifetime_stats(&self) -> &LifetimeCostStats {
        &self.lifetime_stats
    }

    /// Mutable reference to the cumulative lifetime cost statistics.
    pub fn lifetime_stats_mut(&mut self) -> &mut LifetimeCostStats {
        &mut self.lifetime_stats
    }

    /// Grand total lifetime expenditure in USD across all sessions.
    pub fn lifetime_cost(&self) -> f64 {
        self.lifetime_stats.total_breakdown.total_cost
    }

    /// Grand total lifetime tokens consumed across all sessions.
    pub fn lifetime_tokens(&self) -> u64 {
        self.lifetime_stats.total_tokens
    }

    /// Grand total lifetime cost breakdown.
    pub fn lifetime_breakdown(&self) -> CostBreakdown {
        self.lifetime_stats.total_breakdown
    }

    /// Number of turns completed across all sessions in lifetime stats.
    pub fn lifetime_turns_count(&self) -> usize {
        self.lifetime_stats.total_turns
    }

    /// Checks session and lifetime budgets against thresholds (50%, 80%, 100%)
    /// and returns any newly triggered alerts.
    pub fn check_budget_alerts(&mut self) -> Vec<BudgetAlert> {
        let mut new_alerts = Vec::new();

        // 1. Check Session Budget
        if let Some(session_budget) = self.budget_limit_usd {
            if session_budget > 1e-9 {
                let current_cost = self.total_breakdown.total_cost;
                let pct = (current_cost / session_budget) * 100.0;

                let thresholds = [
                    (50.0, BudgetThreshold::FiftyPercent),
                    (80.0, BudgetThreshold::EightyPercent),
                    (100.0, BudgetThreshold::Exceeded),
                ];

                for (thresh_val, thresh_kind) in thresholds {
                    if pct >= thresh_val && !self.triggered_session_alerts.contains(&thresh_kind) {
                        self.triggered_session_alerts.insert(thresh_kind);
                        let alert = BudgetAlert::new(
                            thresh_kind,
                            BudgetScope::Session,
                            current_cost,
                            session_budget,
                        );
                        new_alerts.push(alert);
                    }
                }
            }
        }

        // 2. Check Lifetime Budget
        if let Some(lifetime_budget) = self.lifetime_stats.budget_limit_usd {
            if lifetime_budget > 1e-9 {
                let current_cost = self.lifetime_stats.total_breakdown.total_cost;
                let pct = (current_cost / lifetime_budget) * 100.0;

                let thresholds = [
                    (50.0, BudgetThreshold::FiftyPercent),
                    (80.0, BudgetThreshold::EightyPercent),
                    (100.0, BudgetThreshold::Exceeded),
                ];

                for (thresh_val, thresh_kind) in thresholds {
                    if pct >= thresh_val && !self.triggered_lifetime_alerts.contains(&thresh_kind) {
                        self.triggered_lifetime_alerts.insert(thresh_kind);
                        let alert = BudgetAlert::new(
                            thresh_kind,
                            BudgetScope::Lifetime,
                            current_cost,
                            lifetime_budget,
                        );
                        new_alerts.push(alert);
                    }
                }
            }
        }

        self.recent_alerts = new_alerts.clone();
        new_alerts
    }

    /// Returns alerts generated during the most recent check/turn.
    pub fn recent_alerts(&self) -> &[BudgetAlert] {
        &self.recent_alerts
    }

    /// Clears the recent alerts buffer.
    pub fn clear_recent_alerts(&mut self) {
        self.recent_alerts.clear();
    }

    /// Resets all triggered alert flags so thresholds can fire again.
    pub fn reset_budget_alerts(&mut self) {
        self.triggered_session_alerts.clear();
        self.triggered_lifetime_alerts.clear();
        self.recent_alerts.clear();
    }

    /// Records a completed turn, updates running session and lifetime totals,
    /// and checks budget threshold alerts.
    pub fn record_turn(
        &mut self,
        provider: &str,
        model: &str,
        prompt_tokens: u64,
        completion_tokens: u64,
        cache_read_tokens: u64,
        cache_write_tokens: u64,
    ) -> CostBreakdown {
        let (breakdown, _) = self.record_turn_with_alerts(
            provider,
            model,
            prompt_tokens,
            completion_tokens,
            cache_read_tokens,
            cache_write_tokens,
        );
        breakdown
    }

    /// Records a completed turn and returns both the calculated `CostBreakdown`
    /// and any new `BudgetAlert`s triggered by this turn.
    pub fn record_turn_with_alerts(
        &mut self,
        provider: &str,
        model: &str,
        prompt_tokens: u64,
        completion_tokens: u64,
        cache_read_tokens: u64,
        cache_write_tokens: u64,
    ) -> (CostBreakdown, Vec<BudgetAlert>) {
        let pricing = self.registry.get(provider, model);
        let breakdown = pricing.calculate(
            prompt_tokens,
            completion_tokens,
            cache_read_tokens,
            cache_write_tokens,
        );

        let turn_index = self.turns.len() + 1;
        let record = CostTurnRecord {
            timestamp: Utc::now().to_rfc3339(),
            turn_index,
            provider: pricing.provider.clone(),
            model: pricing.model.clone(),
            prompt_tokens,
            completion_tokens,
            cache_read_tokens,
            cache_write_tokens,
            breakdown,
        };

        // Update session aggregates
        self.total_breakdown += breakdown;
        self.total_tokens = self
            .total_tokens
            .saturating_add(prompt_tokens)
            .saturating_add(completion_tokens);

        *self
            .model_breakdown
            .entry(pricing.model.clone())
            .or_insert_with(CostBreakdown::zero) += breakdown;

        *self
            .provider_breakdown
            .entry(pricing.provider.clone())
            .or_insert_with(CostBreakdown::zero) += breakdown;

        self.turns.push(record);

        // Update lifetime aggregates automatically
        self.lifetime_stats.record_turn(
            &pricing.provider,
            &pricing.model,
            prompt_tokens,
            completion_tokens,
            breakdown,
        );

        // Check budget alerts
        let alerts = self.check_budget_alerts();

        (breakdown, alerts)
    }

    /// Records usage from a `TokenStats` structure for a specific provider and model.
    pub fn record_stats(
        &mut self,
        provider: &str,
        model: &str,
        stats: &TokenStats,
    ) -> CostBreakdown {
        self.record_turn(
            provider,
            model,
            stats.prompt_tokens,
            stats.completion_tokens,
            stats.cache_read_tokens,
            stats.cache_write_tokens,
        )
    }

    /// Records usage from a `TokenStats` structure and returns any triggered budget alerts.
    pub fn record_stats_with_alerts(
        &mut self,
        provider: &str,
        model: &str,
        stats: &TokenStats,
    ) -> (CostBreakdown, Vec<BudgetAlert>) {
        self.record_turn_with_alerts(
            provider,
            model,
            stats.prompt_tokens,
            stats.completion_tokens,
            stats.cache_read_tokens,
            stats.cache_write_tokens,
        )
    }

    /// Starts a new session by archiving the current session into lifetime statistics
    /// and resetting session counters.
    pub fn start_new_session(&mut self) {
        if !self.turns.is_empty() || !self.total_breakdown.is_zero() {
            self.lifetime_stats.session_count += 1;
        }
        self.turns.clear();
        self.model_breakdown.clear();
        self.provider_breakdown.clear();
        self.total_breakdown = CostBreakdown::zero();
        self.total_tokens = 0;
        self.triggered_session_alerts.clear();
        self.recent_alerts.clear();
    }

    /// Grand total estimated expenditure in USD for the current session.
    pub fn total_cost(&self) -> f64 {
        self.total_breakdown.total_cost
    }

    /// Grand total cost breakdown for the current session.
    pub fn total_breakdown(&self) -> CostBreakdown {
        self.total_breakdown
    }

    /// Total tokens consumed across all recorded turns in the current session.
    pub fn total_tokens(&self) -> u64 {
        self.total_tokens
    }

    /// Slice of all individual turn records in the current session.
    pub fn turns(&self) -> &[CostTurnRecord] {
        &self.turns
    }

    /// Per-model aggregate cost breakdown for the current session.
    pub fn model_breakdown(&self) -> &HashMap<String, CostBreakdown> {
        &self.model_breakdown
    }

    /// Per-provider aggregate cost breakdown for the current session.
    pub fn provider_breakdown(&self) -> &HashMap<String, CostBreakdown> {
        &self.provider_breakdown
    }

    /// Resets all accumulated session stats and turn history back to zero.
    pub fn reset(&mut self) {
        self.turns.clear();
        self.model_breakdown.clear();
        self.provider_breakdown.clear();
        self.total_breakdown = CostBreakdown::zero();
        self.total_tokens = 0;
        self.triggered_session_alerts.clear();
        self.recent_alerts.clear();
    }

    /// Resets both session and lifetime statistics back to zero.
    pub fn reset_all(&mut self) {
        self.reset();
        self.lifetime_stats.reset();
        self.triggered_lifetime_alerts.clear();
    }

    /// Generates a human-readable one-line cost summary for the current session.
    pub fn format_summary(&self) -> String {
        format!(
            "Spent: {} across {} turn{} ({} tokens)",
            self.total_breakdown.format_usd(),
            self.turns.len(),
            if self.turns.len() == 1 { "" } else { "s" },
            self.total_tokens
        )
    }

    /// Generates a detailed multi-section cost breakdown report for the current session.
    pub fn format_detailed_report(&self) -> String {
        let mut report = String::new();
        report.push_str("=== Session Cost & Token Usage Report ===\n");
        report.push_str(&format!("Grand Total Cost:  {}\n", self.total_breakdown.format_usd()));
        report.push_str(&format!("Total Tokens:      {}\n", self.total_tokens));
        report.push_str(&format!("Turns Completed:   {}\n", self.turns.len()));

        if self.total_breakdown.cache_savings > 1e-5 {
            report.push_str(&format!(
                "Cache Savings:     {} ({:.1}%)\n",
                format_usd(self.total_breakdown.cache_savings),
                self.total_breakdown.cache_savings_percentage()
            ));
        }

        if let Some(budget) = self.budget_limit_usd {
            let remaining = self.budget_remaining().unwrap_or(0.0);
            let pct = self.session_budget_percentage().unwrap_or(0.0);
            report.push_str(&format!(
                "Session Budget Cap: {} (Remaining: {}, {:.1}% used)\n",
                format_usd(budget),
                format_usd(remaining),
                pct
            ));
        }

        report.push_str("\n--- Breakdown by Model ---\n");
        let mut sorted_models: Vec<_> = self.model_breakdown.iter().collect();
        sorted_models.sort_by(|a, b| b.1.total_cost.partial_cmp(&a.1.total_cost).unwrap_or(std::cmp::Ordering::Equal));
        for (model, cost) in sorted_models {
            report.push_str(&format!(
                "  • {:<30} {:>10} (In: {}, Out: {})\n",
                model,
                cost.format_usd(),
                format_usd(cost.input_cost),
                format_usd(cost.output_cost)
            ));
        }

        report.push_str("\n--- Breakdown by Provider ---\n");
        let mut sorted_provs: Vec<_> = self.provider_breakdown.iter().collect();
        sorted_provs.sort_by(|a, b| b.1.total_cost.partial_cmp(&a.1.total_cost).unwrap_or(std::cmp::Ordering::Equal));
        for (provider, cost) in sorted_provs {
            report.push_str(&format!(
                "  • {:<15} {:>10}\n",
                provider,
                cost.format_usd()
            ));
        }

        report
    }

    /// Generates a comprehensive lifetime report.
    pub fn format_lifetime_report(&self) -> String {
        self.lifetime_stats.format_detailed_report()
    }
}

/// Estimates cost for a given provider, model, and accumulated token stats.
pub fn estimate_cost(provider: &str, model: &str, stats: &TokenStats) -> CostBreakdown {
    let registry = ModelPricingRegistry::new();
    let pricing = registry.get(provider, model);
    pricing.calculate_from_stats(stats)
}

/// Estimates total session cost based on its `active_model` and `token_stats`.
///
/// Automatically resolves model shorthands (e.g. `"sonnet"`, `"r1"`, `"4o"`, `"grok"`)
/// and provider associations.
pub fn estimate_session_cost(session: &Session, default_provider: Option<&str>) -> CostBreakdown {
    let (provider, model) = Config::resolve_model(&session.active_model, default_provider);
    estimate_cost(&provider, &model, session.token_stats())
}

/// Retrieves pricing information for a given provider and model name.
pub fn get_model_pricing(provider: &str, model: &str) -> ModelPricing {
    let registry = ModelPricingRegistry::new();
    registry.get(provider, model)
}

/// Formats a USD amount nicely for user displays.
///
/// - `$0.00` for zero
/// - `< $0.0001` for micro-cents below 0.01 cents
/// - `$0.0042` for small amounts below 1 cent
/// - `$0.125` for amounts between 1 cent and 1 dollar
/// - `$1.25` for amounts above 1 dollar
pub fn format_usd(amount: f64) -> String {
    if amount.abs() < 1e-9 {
        "$0.00".to_string()
    } else if amount < 0.0001 {
        "< $0.0001".to_string()
    } else if amount < 0.01 {
        format!("${:.4}", amount)
    } else if amount < 1.0 {
        format!("${:.3}", amount)
    } else {
        format!("${:.2}", amount)
    }
}

/// Formats a USD amount with exact 6-decimal precision.
pub fn format_usd_precise(amount: f64) -> String {
    format!("${:.6}", amount)
}

/// Formats a concise summary string from a `CostBreakdown`.
pub fn format_cost_summary(breakdown: &CostBreakdown) -> String {
    breakdown.format_summary()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cost_breakdown_arithmetic() {
        let zero = CostBreakdown::zero();
        assert!(zero.is_zero());
        assert_eq!(zero.format_usd(), "$0.00");
        assert_eq!(zero.total_cost, 0.0);

        let c1 = CostBreakdown {
            input_cost: 0.10,
            output_cost: 0.20,
            cache_read_cost: 0.01,
            cache_write_cost: 0.02,
            total_cost: 0.33,
            cache_savings: 0.05,
        };

        let c2 = CostBreakdown {
            input_cost: 0.05,
            output_cost: 0.10,
            cache_read_cost: 0.005,
            cache_write_cost: 0.01,
            total_cost: 0.165,
            cache_savings: 0.02,
        };

        let combined = c1 + c2;
        assert!((combined.input_cost - 0.15).abs() < 1e-6);
        assert!((combined.output_cost - 0.30).abs() < 1e-6);
        assert!((combined.total_cost - 0.495).abs() < 1e-6);
        assert!((combined.cache_savings - 0.07).abs() < 1e-6);
        assert!(combined.cache_savings_percentage() > 0.0);

        let table = combined.format_table();
        assert!(table.contains("Input Cost"));
        assert!(table.contains("Output Cost"));
    }

    #[test]
    fn test_format_usd() {
        assert_eq!(format_usd(0.0), "$0.00");
        assert_eq!(format_usd(0.00004), "< $0.0001");
        assert_eq!(format_usd(0.0042), "$0.0042");
        assert_eq!(format_usd(0.125), "$0.125");
        assert_eq!(format_usd(1.25), "$1.25");
        assert_eq!(format_usd(12.3456), "$12.35");
        assert_eq!(format_usd_precise(0.123456), "$0.123456");
    }

    #[test]
    fn test_claude_pricing_tables() {
        // Claude 3.7 Sonnet
        let p_37 = get_model_pricing("anthropic", "claude-3-7-sonnet-20250219");
        assert_eq!(p_37.input_per_million, 3.00);
        assert_eq!(p_37.output_per_million, 15.00);
        assert_eq!(p_37.cache_read_per_million, 0.30);
        assert_eq!(p_37.cache_write_per_million, 3.75);

        // Alias 3.7
        let p_37_alias = get_model_pricing("anthropic", "3.7-sonnet");
        assert_eq!(p_37_alias.input_per_million, 3.00);

        // Claude 3.5 Sonnet
        let p_35 = get_model_pricing("anthropic", "claude-3-5-sonnet");
        assert_eq!(p_35.input_per_million, 3.00);
        assert_eq!(p_35.output_per_million, 15.00);

        // Claude 3.5 Haiku
        let p_35_h = get_model_pricing("anthropic", "claude-3-5-haiku");
        assert_eq!(p_35_h.input_per_million, 0.80);
        assert_eq!(p_35_h.output_per_million, 4.00);
        assert_eq!(p_35_h.cache_read_per_million, 0.08);
        assert_eq!(p_35_h.cache_write_per_million, 1.00);

        // Claude 3 Haiku
        let p_3_h = get_model_pricing("anthropic", "claude-3-haiku");
        assert_eq!(p_3_h.input_per_million, 0.25);
        assert_eq!(p_3_h.output_per_million, 1.25);

        // Claude 3 Opus
        let p_opus = get_model_pricing("anthropic", "opus");
        assert_eq!(p_opus.input_per_million, 15.00);
        assert_eq!(p_opus.output_per_million, 75.00);
        assert_eq!(p_opus.cache_read_per_million, 1.50);
        assert_eq!(p_opus.cache_write_per_million, 18.75);

        // Calculation with prompt caching
        let cost = p_37.calculate(1_000_000, 100_000, 900_000, 0);
        // Uncached: 100k @ $3/1M = $0.30
        // Cache read: 900k @ $0.30/1M = $0.27
        // Output: 100k @ $15/1M = $1.50
        // Total = 0.30 + 0.27 + 1.50 = $2.07
        // Savings = 900k * ($3.00 - $0.30)/1M = $2.43
        assert!((cost.input_cost - 0.30).abs() < 1e-6);
        assert!((cost.cache_read_cost - 0.27).abs() < 1e-6);
        assert!((cost.output_cost - 1.50).abs() < 1e-6);
        assert!((cost.total_cost - 2.07).abs() < 1e-6);
        assert!((cost.cache_savings - 2.43).abs() < 1e-6);
    }

    #[test]
    fn test_openai_pricing_tables() {
        // GPT-4.5
        let p_45 = get_model_pricing("openai", "gpt-4.5-preview");
        assert_eq!(p_45.input_per_million, 75.00);
        assert_eq!(p_45.output_per_million, 150.00);
        assert_eq!(p_45.cache_read_per_million, 37.50);

        // GPT-4o
        let p_4o = get_model_pricing("openai", "gpt-4o");
        assert_eq!(p_4o.input_per_million, 2.50);
        assert_eq!(p_4o.output_per_million, 10.00);
        assert_eq!(p_4o.cache_read_per_million, 1.25);

        // GPT-4o Mini
        let p_mini = get_model_pricing("openai", "gpt-4o-mini");
        assert_eq!(p_mini.input_per_million, 0.15);
        assert_eq!(p_mini.output_per_million, 0.60);
        assert_eq!(p_mini.cache_read_per_million, 0.075);

        // o3-mini
        let p_o3 = get_model_pricing("openai", "o3-mini");
        assert_eq!(p_o3.input_per_million, 1.10);
        assert_eq!(p_o3.output_per_million, 4.40);
        assert_eq!(p_o3.cache_read_per_million, 0.55);

        // o1
        let p_o1 = get_model_pricing("openai", "o1");
        assert_eq!(p_o1.input_per_million, 15.00);
        assert_eq!(p_o1.output_per_million, 60.00);
        assert_eq!(p_o1.cache_read_per_million, 7.50);

        // o1-mini
        let p_o1_mini = get_model_pricing("openai", "o1-mini");
        assert_eq!(p_o1_mini.input_per_million, 1.10);
        assert_eq!(p_o1_mini.output_per_million, 4.40);

        // GPT-4 Turbo
        let p_turbo = get_model_pricing("openai", "gpt-4-turbo");
        assert_eq!(p_turbo.input_per_million, 10.00);
        assert_eq!(p_turbo.output_per_million, 30.00);

        // GPT-4 Legacy
        let p_legacy = get_model_pricing("openai", "gpt-4");
        assert_eq!(p_legacy.input_per_million, 30.00);
        assert_eq!(p_legacy.output_per_million, 60.00);

        // GPT-3.5 Turbo
        let p_35_turbo = get_model_pricing("openai", "gpt-3.5-turbo");
        assert_eq!(p_35_turbo.input_per_million, 0.50);
        assert_eq!(p_35_turbo.output_per_million, 1.50);

        // Embeddings
        let p_embed = get_model_pricing("openai", "text-embedding-3-small");
        assert_eq!(p_embed.input_per_million, 0.02);
    }

    #[test]
    fn test_deepseek_pricing_tables() {
        // DeepSeek Chat / V3
        let p_chat = get_model_pricing("deepseek", "deepseek-chat");
        assert_eq!(p_chat.input_per_million, 0.14);
        assert_eq!(p_chat.output_per_million, 0.28);
        assert_eq!(p_chat.cache_read_per_million, 0.014);

        // DeepSeek Reasoner / R1
        let p_r1 = get_model_pricing("deepseek", "deepseek-reasoner");
        assert_eq!(p_r1.input_per_million, 0.55);
        assert_eq!(p_r1.output_per_million, 2.19);
        assert_eq!(p_r1.cache_read_per_million, 0.14);

        // Calculation test: 1M tokens with 800k cache hit on R1
        let cost = p_r1.calculate(1_000_000, 100_000, 800_000, 0);
        // Uncached: 200k * 0.55 / 1M = 0.11
        // Cache read: 800k * 0.14 / 1M = 0.112
        // Output: 100k * 2.19 / 1M = 0.219
        // Total = 0.11 + 0.112 + 0.219 = 0.441
        assert!((cost.input_cost - 0.11).abs() < 1e-6);
        assert!((cost.cache_read_cost - 0.112).abs() < 1e-6);
        assert!((cost.output_cost - 0.219).abs() < 1e-6);
        assert!((cost.total_cost - 0.441).abs() < 1e-6);
    }

    #[test]
    fn test_xai_pricing_tables() {
        // Grok-2
        let p_grok2 = get_model_pricing("xai", "grok-2");
        assert_eq!(p_grok2.input_per_million, 2.00);
        assert_eq!(p_grok2.output_per_million, 10.00);
        assert_eq!(p_grok2.cache_read_per_million, 1.00);

        // Grok-3
        let p_grok3 = get_model_pricing("xai", "grok-3");
        assert_eq!(p_grok3.input_per_million, 3.00);
        assert_eq!(p_grok3.output_per_million, 15.00);

        // Grok-3 Mini
        let p_grok3_mini = get_model_pricing("xai", "grok-3-mini");
        assert_eq!(p_grok3_mini.input_per_million, 0.30);
        assert_eq!(p_grok3_mini.output_per_million, 1.50);
    }

    #[test]
    fn test_openrouter_pricing_tables() {
        // Sub-provider prefix resolution
        let p_router_claude = get_model_pricing("openrouter", "anthropic/claude-3.5-sonnet");
        assert_eq!(p_router_claude.input_per_million, 3.00);
        assert_eq!(p_router_claude.output_per_million, 15.00);

        let p_router_r1 = get_model_pricing("openrouter", "deepseek/deepseek-r1");
        assert_eq!(p_router_r1.input_per_million, 0.55);
        assert_eq!(p_router_r1.output_per_million, 2.19);

        let p_router_4o = get_model_pricing("openrouter", "openai/gpt-4o");
        assert_eq!(p_router_4o.input_per_million, 2.50);

        // Open-weights models
        let p_llama = get_model_pricing("openrouter", "meta-llama/llama-3.3-70b-instruct");
        assert_eq!(p_llama.input_per_million, 0.12);
        assert_eq!(p_llama.output_per_million, 0.30);

        let p_qwen = get_model_pricing("openrouter", "qwen/qwen-2.5-coder-32b-instruct");
        assert_eq!(p_qwen.input_per_million, 0.07);
        assert_eq!(p_qwen.output_per_million, 0.16);

        let p_mistral = get_model_pricing("openrouter", "mistralai/mistral-large-2411");
        assert_eq!(p_mistral.input_per_million, 2.00);
        assert_eq!(p_mistral.output_per_million, 6.00);
    }

    #[test]
    fn test_budget_threshold_warning_alerts() {
        let mut tracker = CostTracker::new();
        // Set session budget to $1.00
        tracker.set_session_budget(1.00);
        assert_eq!(tracker.budget_limit(), Some(1.00));
        assert_eq!(tracker.budget_remaining(), Some(1.00));
        assert_eq!(tracker.session_budget_percentage(), Some(0.0));

        // Turn 1: Spend $0.40 (40% - No alert yet)
        // Using Claude 3.5 Sonnet ($3 in, $15 out): 100k prompt = $0.30, 6667 tokens @ $15/1M = $0.10 => $0.40
        let (_cost1, alerts1) = tracker.record_turn_with_alerts(
            "anthropic",
            "claude-3-5-sonnet",
            100_000,
            6_667,
            0,
            0,
        );
        assert_eq!(alerts1.len(), 0);
        assert!(!tracker.is_budget_exceeded());

        // Turn 2: Spend another $0.15 (Total $0.55 = 55% -> triggers 50% warning)
        // 50k prompt = $0.15
        let (_cost2, alerts2) = tracker.record_turn_with_alerts(
            "anthropic",
            "claude-3-5-sonnet",
            50_000,
            0,
            0,
            0,
        );
        assert_eq!(alerts2.len(), 1);
        assert_eq!(alerts2[0].threshold, BudgetThreshold::FiftyPercent);
        assert_eq!(alerts2[0].scope, BudgetScope::Session);
        assert!(alerts2[0].is_warning());
        assert!(!alerts2[0].is_critical());
        assert!(alerts2[0].message.contains("50%"));

        // Turn 3: Spend another $0.10 (Total $0.65 = 65% -> 50% was already fired, no new alerts)
        let (_cost3, alerts3) = tracker.record_turn_with_alerts(
            "anthropic",
            "claude-3-5-sonnet",
            33_334,
            0,
            0,
            0,
        );
        assert_eq!(alerts3.len(), 0);

        // Turn 4: Spend another $0.20 (Total $0.85 = 85% -> triggers 80% alert)
        let (_cost4, alerts4) = tracker.record_turn_with_alerts(
            "anthropic",
            "claude-3-5-sonnet",
            66_667,
            0,
            0,
            0,
        );
        assert_eq!(alerts4.len(), 1);
        assert_eq!(alerts4[0].threshold, BudgetThreshold::EightyPercent);
        assert!(alerts4[0].message.contains("80%"));

        // Turn 5: Spend another $0.20 (Total $1.05 = 105% -> triggers 100% Exceeded alert)
        let (_cost5, alerts5) = tracker.record_turn_with_alerts(
            "anthropic",
            "claude-3-5-sonnet",
            66_667,
            0,
            0,
            0,
        );
        assert_eq!(alerts5.len(), 1);
        assert_eq!(alerts5[0].threshold, BudgetThreshold::Exceeded);
        assert!(alerts5[0].is_critical());
        assert!(tracker.is_budget_exceeded());
        assert_eq!(tracker.budget_remaining(), Some(0.0));
    }

    #[test]
    fn test_lifetime_cost_accumulation() {
        let mut tracker = CostTracker::new();
        tracker.set_session_budget(2.00);
        tracker.set_lifetime_budget(10.00);

        // Session 1: 2 turns
        tracker.record_turn("openai", "gpt-4o", 100_000, 10_000, 0, 0); // 100k*2.5/1M + 10k*10/1M = 0.25 + 0.10 = 0.35
        tracker.record_turn("deepseek", "deepseek-chat", 500_000, 50_000, 0, 0); // 500k*0.14/1M + 50k*0.28/1M = 0.07 + 0.014 = 0.084
        assert!((tracker.total_cost() - 0.434).abs() < 1e-6);
        assert!((tracker.lifetime_cost() - 0.434).abs() < 1e-6);
        assert_eq!(tracker.lifetime_tokens(), 100_000 + 10_000 + 500_000 + 50_000);

        // Start Session 2:
        tracker.start_new_session();
        assert_eq!(tracker.turns().len(), 0);
        assert_eq!(tracker.total_cost(), 0.0);
        assert_eq!(tracker.lifetime_stats().session_count, 1);
        assert!((tracker.lifetime_cost() - 0.434).abs() < 1e-6);

        // Session 2: 1 turn
        tracker.record_turn("anthropic", "claude-3-7-sonnet", 200_000, 20_000, 0, 0); // 200k*3/1M + 20k*15/1M = 0.60 + 0.30 = 0.90
        assert!((tracker.total_cost() - 0.90).abs() < 1e-6);
        assert!((tracker.lifetime_cost() - (0.434 + 0.90)).abs() < 1e-6);
        assert_eq!(tracker.lifetime_turns_count(), 3);

        // Test Lifetime report
        let report = tracker.format_lifetime_report();
        assert!(report.contains("Grand Total Lifetime Cost"));
        assert!(report.contains("gpt-4o"));
        assert!(report.contains("deepseek-chat"));
        assert!(report.contains("claude-3-7-sonnet"));
    }

    #[test]
    fn test_custom_pricing_override() {
        let mut registry = ModelPricingRegistry::new();
        let custom = ModelPricing::new("custom_corp", "custom-model", 0.50, 1.00, 0.10, 0.50);
        registry.register(custom);

        let retrieved = registry.get("custom_corp", "custom-model");
        assert_eq!(retrieved.input_per_million, 0.50);
        assert_eq!(retrieved.output_per_million, 1.00);
    }

    #[test]
    fn test_estimate_session_cost() {
        let mut session = Session::new("anthropic:claude-3-5-sonnet");
        session.record_usage(500_000, 50_000);

        let cost = estimate_session_cost(&session, None);
        assert!((cost.input_cost - 1.50).abs() < 1e-6);
        assert!((cost.output_cost - 0.75).abs() < 1e-6);
        assert!((cost.total_cost - 2.25).abs() < 1e-6);
        assert_eq!(cost.format_usd(), "$2.25");
    }
}
