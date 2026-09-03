use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

/// Per-model cache savings item returned by `/v1/usage`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ModelCacheSavings {
    pub cached_tokens: u64,
    pub savings_usd: f64,
}

/// Deserializer helper to handle both numeric float values and structured objects
/// for `cache_savings_by_model`.
fn deserialize_cache_savings_by_model<'de, D>(
    deserializer: D,
) -> Result<HashMap<String, ModelCacheSavings>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde_json::Value;
    let map: HashMap<String, Value> = HashMap::deserialize(deserializer)?;
    let mut result = HashMap::new();
    for (k, v) in &map {
        match v {
            Value::Number(num) => {
                let savings = num.as_f64().unwrap_or(0.0);
                result.insert(
                    k.clone(),
                    ModelCacheSavings {
                        cached_tokens: 0,
                        savings_usd: savings,
                    },
                );
            }
            Value::Object(obj) => {
                let cached_tokens = obj
                    .get("cached_tokens")
                    .and_then(|t| t.as_u64())
                    .unwrap_or(0);
                let savings_usd = obj
                    .get("savings_usd")
                    .and_then(|s| s.as_f64())
                    .unwrap_or(0.0);
                result.insert(
                    k.clone(),
                    ModelCacheSavings {
                        cached_tokens,
                        savings_usd,
                    },
                );
            }
            _ => {}
        }
    }
    Ok(result)
}

/// Backend account usage and quota report from `/v1/usage`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct BackendUsageReport {
    pub user_email: Option<String>,
    pub plan_name: String,
    pub used_usd: f64,
    pub monthly_limit_usd: f64,
    pub remaining_usd: f64,
    pub used_tokens_this_month: u64,
    pub cached_tokens_this_month: u64,
    pub prompt_tokens_this_month: u64,
    pub prompt_cache_hit_rate_pct: f64,
    pub cache_hit_count_this_month: u64,
    pub cache_savings_usd_this_month: f64,
    #[serde(deserialize_with = "deserialize_cache_savings_by_model")]
    pub cache_savings_by_model: HashMap<String, ModelCacheSavings>,
    pub is_payg: bool,
}
impl BackendUsageReport {
    /// Percentage of monthly limit consumed (0.0 - 100.0+).
    pub fn usage_percentage(&self) -> f64 {
        if self.monthly_limit_usd > 0.0 {
            (self.used_usd / self.monthly_limit_usd) * 100.0
        } else {
            0.0
        }
    }

    /// Whether usage has reached or exceeded 80% of limit.
    pub fn is_near_limit(&self) -> bool {
        self.usage_percentage() >= 80.0
    }

    /// Whether usage has reached or exceeded 100% of limit.
    pub fn is_over_limit(&self) -> bool {
        self.monthly_limit_usd > 0.0 && self.used_usd >= self.monthly_limit_usd
    }
}

/// Normalizes the base URL into an endpoint ending with `/usage`.
/// Strips trailing slashes and handles URLs that already end with `/usage`.
pub fn normalize_usage_url(base_url: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.ends_with("/usage") {
        trimmed.to_string()
    } else {
        format!("{}/usage", trimmed)
    }
}

/// Fetches backend usage with an existing `reqwest::Client`.
pub async fn fetch_backend_usage_with_client(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
) -> anyhow::Result<BackendUsageReport> {
    let url = normalize_usage_url(base_url);

    let auth_val = if api_key.starts_with("Bearer ") {
        api_key.to_string()
    } else {
        format!("Bearer {}", api_key.trim())
    };

    let resp = client
        .get(&url)
        .header(reqwest::header::AUTHORIZATION, auth_val)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to connect to Fusion backend: {}", e))?;

    let status = resp.status();
    if !status.is_success() {
        let error_body = resp.text().await.unwrap_or_default();
        anyhow::bail!(
            "Fusion usage API returned HTTP {}: {}",
            status,
            error_body.trim()
        );
    }

    let text = resp
        .text()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to read response body: {}", e))?;

    let report: BackendUsageReport = serde_json::from_str(&text).map_err(|e| {
        anyhow::anyhow!(
            "Failed to parse usage response JSON: {}. Raw response: {}",
            e,
            text
        )
    })?;

    Ok(report)
}

/// Fetches the backend usage report from `{base_url}/usage` using the provided API key.
/// Standardizes base_url by stripping trailing slashes and appending `/usage` if needed.
/// Uses a 10s request timeout.
pub async fn fetch_backend_usage(
    base_url: &str,
    api_key: &str,
) -> anyhow::Result<BackendUsageReport> {
    let client = reqwest::Client::builder()
        .use_rustls_tls()
        .timeout(Duration::from_secs(10))
        .connect_timeout(Duration::from_secs(5))
        .user_agent("Fusion-AI/2.0.0")
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());

    fetch_backend_usage_with_client(&client, base_url, api_key).await
}

/// Synchronous wrapper for `fetch_backend_usage` that safely runs in or outside a tokio runtime.
pub fn fetch_backend_usage_sync(
    base_url: &str,
    api_key: &str,
) -> anyhow::Result<BackendUsageReport> {
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        tokio::task::block_in_place(|| handle.block_on(fetch_backend_usage(base_url, api_key)))
    } else {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        rt.block_on(fetch_backend_usage(base_url, api_key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_usage_url() {
        assert_eq!(
            normalize_usage_url("https://api.fusioncode.app/v1"),
            "https://api.fusioncode.app/v1/usage"
        );
    }

    #[test]
    fn test_deserialize_real_backend_json_with_nested_cache_savings() {
        let raw = r#"{
            "user_email":"aungmyatmoe834@gmail.com",
            "plan_name":"Admin",
            "used_usd":0.28259528999999983,
            "monthly_limit_usd":100,
            "remaining_usd":99.7174,
            "used_tokens_this_month":4464222,
            "cached_tokens_this_month":3465283,
            "prompt_tokens_this_month":4410108,
            "prompt_cache_hit_rate_pct":44,
            "cache_hit_count_this_month":223,
            "cache_savings_usd_this_month":0.620197,
            "cache_savings_by_model":{
                "MiniMaxAI/MiniMax-M2.7":{"cached_tokens":2193211,"savings_usd":0.504439},
                "deepseek-ai/DeepSeek-V4-Flash-0731":{"cached_tokens":1272072,"savings_usd":0.115759},
                "minimax":{"cached_tokens":0,"savings_usd":0},
                "moonshotai/Kimi-K2.6":{"cached_tokens":0,"savings_usd":0}
            },
            "is_payg":true
        }"#;
        let report: BackendUsageReport =
            serde_json::from_str(raw).expect("parse real backend json");
        assert_eq!(
            report.user_email.as_deref(),
            Some("aungmyatmoe834@gmail.com")
        );
        assert_eq!(report.plan_name, "Admin");
        assert!(report.is_payg);
        assert_eq!(report.cache_savings_by_model.len(), 4);
        let minimax = report
            .cache_savings_by_model
            .get("MiniMaxAI/MiniMax-M2.7")
            .unwrap();
        assert_eq!(minimax.cached_tokens, 2193211);
        assert_eq!(minimax.savings_usd, 0.504439);
    }

    #[test]
    fn test_deserialize_full_json() {
        let raw_json = r#"{
            "user_email": "test@example.com",
            "plan_name": "Pro",
            "used_usd": 15.75,
            "monthly_limit_usd": 50.0,
            "remaining_usd": 34.25,
            "used_tokens_this_month": 1250000,
            "cached_tokens_this_month": 800000,
            "prompt_tokens_this_month": 1000000,
            "prompt_cache_hit_rate_pct": 80.0,
            "cache_hit_count_this_month": 420,
            "cache_savings_usd_this_month": 4.85,
            "cache_savings_by_model": {
                "claude-3-5-sonnet": 3.20,
                "claude-3-7-sonnet": 1.65
            },
            "is_payg": false
        }"#;
        let report: BackendUsageReport =
            serde_json::from_str(raw_json).expect("should deserialize full json");
        assert_eq!(report.user_email, Some("test@example.com".to_string()));
        assert_eq!(report.plan_name, "Pro");
        assert!((report.used_usd - 15.75).abs() < f64::EPSILON);
        assert!((report.monthly_limit_usd - 50.0).abs() < f64::EPSILON);
        assert!((report.remaining_usd - 34.25).abs() < f64::EPSILON);
        assert_eq!(report.used_tokens_this_month, 1250000);
        assert_eq!(report.cached_tokens_this_month, 800000);
        assert_eq!(report.prompt_tokens_this_month, 1000000);
        assert!((report.prompt_cache_hit_rate_pct - 80.0).abs() < f64::EPSILON);
        assert_eq!(report.cache_hit_count_this_month, 420);
        assert!((report.cache_savings_usd_this_month - 4.85).abs() < f64::EPSILON);
        assert_eq!(
            report
                .cache_savings_by_model
                .get("claude-3-5-sonnet")
                .map(|s| s.savings_usd),
            Some(3.20)
        );
        assert_eq!(
            report
                .cache_savings_by_model
                .get("claude-3-7-sonnet")
                .map(|s| s.savings_usd),
            Some(1.65)
        );
        assert!(!report.is_near_limit());
        assert!(!report.is_over_limit());
    }

    #[test]
    fn test_deserialize_partial_json_with_defaults() {
        let partial_json = r#"{
            "plan_name": "Free",
            "used_usd": 0.50
        }"#;

        let report: BackendUsageReport =
            serde_json::from_str(partial_json).expect("should deserialize with defaults");
        assert_eq!(report.user_email, None);
        assert_eq!(report.plan_name, "Free");
        assert!((report.used_usd - 0.50).abs() < f64::EPSILON);
        assert_eq!(report.monthly_limit_usd, 0.0);
        assert_eq!(report.remaining_usd, 0.0);
        assert_eq!(report.used_tokens_this_month, 0);
        assert_eq!(report.cached_tokens_this_month, 0);
        assert_eq!(report.prompt_tokens_this_month, 0);
        assert_eq!(report.prompt_cache_hit_rate_pct, 0.0);
        assert_eq!(report.cache_hit_count_this_month, 0);
        assert_eq!(report.cache_savings_usd_this_month, 0.0);
        assert_eq!(report.cache_savings_by_model.len(), 0);
        assert_eq!(report.is_payg, false);
    }

    #[test]
    fn test_serialization_roundtrip() {
        let mut report = BackendUsageReport::default();
        report.user_email = Some("coder@fusion.dev".to_string());
        report.plan_name = "Team".to_string();
        report.used_usd = 85.0;
        report.monthly_limit_usd = 100.0;
        report.remaining_usd = 15.0;
        report.cache_savings_by_model.insert(
            "claude-3-7-sonnet".to_string(),
            ModelCacheSavings {
                cached_tokens: 50000,
                savings_usd: 5.50,
            },
        );
        assert!(report.is_near_limit());
        assert!(!report.is_over_limit());

        let serialized = serde_json::to_string(&report).expect("serialization works");
        let deserialized: BackendUsageReport =
            serde_json::from_str(&serialized).expect("deserialization works");
        assert_eq!(report, deserialized);
    }
}
