//! Integration test for /usage command and Fusion Backend /v1/usage contract.

use fusion::provider::usage::{fetch_backend_usage, BackendUsageReport, ModelCacheSavings};
use fusion::ui::slash::SlashCommand;
use fusion::ui::usage_card::render_backend_usage_fx;
use std::collections::HashMap;

#[test]
fn test_slash_command_usage_route() {
    assert_eq!(SlashCommand::parse("/usage"), Some(SlashCommand::Usage));
    assert_eq!(SlashCommand::parse("/stats"), Some(SlashCommand::Stats));
    assert_eq!(SlashCommand::parse("/cost"), Some(SlashCommand::Stats));
}

#[test]
fn test_backend_usage_full_render() {
    let mut model_savings = HashMap::new();
    model_savings.insert(
        "deepseek-ai/DeepSeek-V4-Flash-0731".to_string(),
        ModelCacheSavings {
            cached_tokens: 1_272_072,
            savings_usd: 0.4215,
        },
    );
    model_savings.insert(
        "MiniMaxAI/MiniMax-M2.7".to_string(),
        ModelCacheSavings {
            cached_tokens: 2_193_211,
            savings_usd: 0.1850,
        },
    );
    let report = BackendUsageReport {
        user_email: Some("dev@fusioncode.app".to_string()),
        plan_name: "Pro Builder".to_string(),
        used_usd: 12.45,
        monthly_limit_usd: 50.00,
        remaining_usd: 37.55,
        used_tokens_this_month: 2_450_000,
        cached_tokens_this_month: 1_280_000,
        prompt_tokens_this_month: 1_850_000,
        prompt_cache_hit_rate_pct: 69.19,
        cache_hit_count_this_month: 412,
        cache_savings_usd_this_month: 0.6065,
        cache_savings_by_model: model_savings,
        is_payg: false,
    };

    let rendered = render_backend_usage_fx(&report);
    assert!(rendered.contains("Pro Builder"));
    assert!(rendered.contains("dev@fusioncode.app"));
    assert!(rendered.contains("$12.45"));
    assert!(rendered.contains("$50.00"));
    assert!(rendered.contains("$37.55"));
    assert!(rendered.contains("69.2%"));
}
