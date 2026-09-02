use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::Config;

/// The legacy schema version used before explicit version tracking was introduced.
pub const LEGACY_SCHEMA_VERSION: u32 = 0;

/// The v1 schema version (initial explicit schema).
pub const V1_SCHEMA_VERSION: u32 = 1;

/// The current configuration schema version in Fusion v2.
pub const CURRENT_SCHEMA_VERSION: u32 = 2;

/// Outcome report produced after a configuration migration check or execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationOutcome {
    /// Schema version the configuration originated from.
    pub from_version: u32,
    /// Target schema version after migration.
    pub to_version: u32,
    /// Whether any schema migration or data transformation was performed.
    pub performed_migration: bool,
    /// Path to the backup file created prior to writing changes, if applicable.
    pub backup_path: Option<PathBuf>,
    /// Chronological list of human-readable changes applied during migration.
    pub changes: Vec<String>,
}

/// Errors that can occur during configuration schema detection, migration, or persistence.
#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
    #[error("Failed to parse config JSON: {0}")]
    JsonParse(#[from] serde_json::Error),

    #[error("IO error during config migration: {0}")]
    Io(#[from] std::io::Error),

    #[error("Config schema version {version} is newer than maximum supported version {max_supported}")]
    FutureVersion { version: u32, max_supported: u32 },

    #[error("Config schema migration step from v{from} to v{to} failed: {reason}")]
    StepFailed { from: u32, to: u32, reason: String },

    #[error("Config JSON root must be an object, found: {0}")]
    InvalidRootType(String),
}

/// Detects the schema version of a parsed JSON configuration value.
///
/// Returns `0` (legacy) if no version or schema_version field is present.
pub fn detect_version(val: &Value) -> u32 {
    if let Some(v) = val.get("version").and_then(|v| v.as_u64()) {
        return v as u32;
    }
    if let Some(v) = val.get("schema_version").and_then(|v| v.as_u64()) {
        return v as u32;
    }
    LEGACY_SCHEMA_VERSION
}

/// Migrates an unversioned (v0/legacy) configuration object into the standardized v1 schema.
///
/// Handles:
/// - Legacy field aliases: `provider` -> `default_provider`, `model` -> `default_model`,
///   `temperature` / `temp` -> `default_temperature`, `tokens` / `max_token` -> `max_tokens`
/// - Provider-specific key aliases: `openai_key` -> `openai_api_key`, `claude_api_key` -> `anthropic_api_key`, etc.
/// - Provider-specific URL aliases: `openai_url` -> `openai_base_url`, `ollama_url` / `ollama_host` -> `ollama_base_url`, etc.
/// - Generic API key & URL mappings (`api_key`, `base_url`)
/// - Advisor settings aliases: `advisors` / `enable_advisors` -> `advisors_enabled`
/// - Type coercions (string booleans, string integers/floats)
pub fn migrate_v0_to_v1(val: &mut Value, changes: &mut Vec<String>) -> Result<(), MigrationError> {
    if !val.is_object() {
        return Err(MigrationError::InvalidRootType(format!("{:?}", val)));
    }
    let obj = val.as_object_mut().unwrap();

    // 1. Provider alias migration
    if !obj.contains_key("default_provider") {
        for alias in &["provider", "model_provider", "defaultProvider"] {
            if let Some(old_val) = obj.remove(*alias) {
                obj.insert("default_provider".to_string(), old_val);
                changes.push(format!("Renamed legacy field '{}' to 'default_provider'", alias));
                break;
            }
        }
    }

    // 2. Model alias migration
    if !obj.contains_key("default_model") {
        for alias in &["model", "selected_model", "defaultModel"] {
            if let Some(old_val) = obj.remove(*alias) {
                obj.insert("default_model".to_string(), old_val);
                changes.push(format!("Renamed legacy field '{}' to 'default_model'", alias));
                break;
            }
        }
    }

    // 3. Temperature alias migration & type coercion
    if !obj.contains_key("default_temperature") {
        for alias in &["temperature", "temp", "defaultTemperature"] {
            if let Some(old_val) = obj.remove(*alias) {
                obj.insert("default_temperature".to_string(), old_val);
                changes.push(format!("Renamed legacy field '{}' to 'default_temperature'", alias));
                break;
            }
        }
    }
    if let Some(temp_val) = obj.get_mut("default_temperature") {
        if let Some(s) = temp_val.as_str().map(|s| s.to_string()) {
            if let Ok(f) = s.trim().parse::<f64>() {
                *temp_val = Value::from(f);
                changes.push(format!("Coerced string temperature \"{}\" to float {}", s, f));
            }
        }
    }

    // 4. Max tokens alias migration & type coercion
    if !obj.contains_key("max_tokens") {
        for alias in &["tokens", "max_token", "maxTokens", "max_tokens_limit"] {
            if let Some(old_val) = obj.remove(*alias) {
                obj.insert("max_tokens".to_string(), old_val);
                changes.push(format!("Renamed legacy field '{}' to 'max_tokens'", alias));
                break;
            }
        }
    }
    if let Some(tok_val) = obj.get_mut("max_tokens") {
        if let Some(s) = tok_val.as_str().map(|s| s.to_string()) {
            if let Ok(i) = s.trim().parse::<u64>() {
                *tok_val = Value::from(i);
                changes.push(format!("Coerced string max_tokens \"{}\" to integer {}", s, i));
            }
        }
    }

    // 5. Provider API key aliases
    let key_aliases = [
        ("openai_api_key", &["openai_key", "open_ai_api_key", "openAiApiKey"][..]),
        ("anthropic_api_key", &["anthropic_key", "claude_api_key", "claude_key", "anthropicApiKey"]),
        ("deepseek_api_key", &["deepseek_key", "deep_seek_api_key", "deepSeekApiKey"]),
        ("xai_api_key", &["xai_key", "grok_api_key", "grok_key", "xaiApiKey"]),
        ("openrouter_api_key", &["openrouter_key", "open_router_api_key", "openRouterApiKey"]),
    ];

    for (canonical, aliases) in &key_aliases {
        if !obj.contains_key(*canonical) {
            for alias in *aliases {
                if let Some(v) = obj.remove(*alias) {
                    obj.insert(canonical.to_string(), v);
                    changes.push(format!("Renamed legacy API key field '{}' to '{}'", alias, canonical));
                    break;
                }
            }
        }
    }

    // 6. Provider Base URL aliases
    let url_aliases = [
        ("openai_base_url", &["openai_url", "openai_endpoint", "openAiBaseUrl"][..]),
        ("anthropic_base_url", &["anthropic_url", "anthropic_endpoint", "anthropicBaseUrl"]),
        ("deepseek_base_url", &["deepseek_url", "deepseek_endpoint", "deepSeekBaseUrl"]),
        ("xai_base_url", &["xai_url", "grok_url", "grok_endpoint", "xaiBaseUrl"]),
        ("openrouter_base_url", &["openrouter_url", "openrouter_endpoint", "openRouterBaseUrl"]),
        ("ollama_base_url", &["ollama_url", "ollama_host", "ollama_endpoint", "ollamaBaseUrl"]),
    ];

    for (canonical, aliases) in &url_aliases {
        if !obj.contains_key(*canonical) {
            for alias in *aliases {
                if let Some(v) = obj.remove(*alias) {
                    obj.insert(canonical.to_string(), v);
                    changes.push(format!("Renamed legacy base URL field '{}' to '{}'", alias, canonical));
                    break;
                }
            }
        }
    }

    // 7. Generic API key mapping
    if let Some(generic_key) = obj.remove("api_key").or_else(|| obj.remove("llm_api_key")) {
        if let Some(k_str) = generic_key.as_str() {
            if !k_str.trim().is_empty() {
                let provider = obj.get("default_provider")
                    .and_then(|p| p.as_str())
                    .unwrap_or("deepseek")
                    .to_lowercase();

                let target_field = match provider.as_str() {
                    "openai" => "openai_api_key",
                    "anthropic" | "claude" => "anthropic_api_key",
                    "xai" | "grok" => "xai_api_key",
                    "openrouter" => "openrouter_api_key",
                    _ => "deepseek_api_key",
                };

                if !obj.contains_key(target_field) {
                    obj.insert(target_field.to_string(), Value::String(k_str.trim().to_string()));
                    changes.push(format!("Mapped generic 'api_key' to '{}'", target_field));
                }
            }
        }
    }

    // 8. Generic Base URL mapping
    if let Some(generic_url) = obj.remove("base_url").or_else(|| obj.remove("api_base")) {
        if let Some(u_str) = generic_url.as_str() {
            if !u_str.trim().is_empty() {
                let provider = obj.get("default_provider")
                    .and_then(|p| p.as_str())
                    .unwrap_or("deepseek")
                    .to_lowercase();

                let target_field = match provider.as_str() {
                    "openai" => "openai_base_url",
                    "anthropic" | "claude" => "anthropic_base_url",
                    "xai" | "grok" => "xai_base_url",
                    "openrouter" => "openrouter_base_url",
                    "ollama" => "ollama_base_url",
                    _ => "deepseek_base_url",
                };

                if !obj.contains_key(target_field) {
                    obj.insert(target_field.to_string(), Value::String(u_str.trim().to_string()));
                    changes.push(format!("Mapped generic 'base_url' to '{}'", target_field));
                }
            }
        }
    }

    // 9. Advisor aliases and boolean coercion
    if !obj.contains_key("advisors_enabled") {
        for alias in &["advisors", "enable_advisors", "advisorsEnabled"] {
            if let Some(old_val) = obj.remove(*alias) {
                obj.insert("advisors_enabled".to_string(), old_val);
                changes.push(format!("Renamed legacy field '{}' to 'advisors_enabled'", alias));
                break;
            }
        }
    }
    if let Some(adv_val) = obj.get_mut("advisors_enabled") {
        if let Some(s) = adv_val.as_str().map(|s| s.to_string()) {
            let lower = s.trim().to_lowercase();
            let b = matches!(lower.as_str(), "true" | "1" | "yes" | "on");
            *adv_val = Value::Bool(b);
            changes.push(format!("Coerced string advisors_enabled \"{}\" to boolean {}", s, b));
        }
    }

    // 10. Tag as version 1
    obj.insert("version".to_string(), Value::from(V1_SCHEMA_VERSION));
    changes.push("Upgraded schema from v0 (legacy) to v1".to_string());

    Ok(())
}

/// Migrates a v1 configuration object into the current v2 schema.
///
/// Handles:
/// - Base URL normalization (stripping trailing slashes, ensuring http/https protocol)
/// - Sane Ollama fallback URL if missing or null
/// - API key sanitization (trimming, stripping surrounding quotes, removing placeholder templates)
/// - Default provider case normalization (lowercase)
/// - Shorthand model canonicalization
/// - Temperature range validation & clamping
/// - Adding schema version metadata: `"version": 2`
pub fn migrate_v1_to_v2(val: &mut Value, changes: &mut Vec<String>) -> Result<(), MigrationError> {
    if !val.is_object() {
        return Err(MigrationError::InvalidRootType(format!("{:?}", val)));
    }
    let obj = val.as_object_mut().unwrap();

    // 1. Normalize provider name
    if let Some(prov_val) = obj.get_mut("default_provider") {
        if let Some(p) = prov_val.as_str().map(|s| s.to_string()) {
            let lower = p.trim().to_lowercase();
            if lower != p {
                *prov_val = Value::String(lower.clone());
                changes.push(format!("Normalized default_provider '{}' to lowercase '{}'", p, lower));
            }
        }
    }

    // 2. Canonicalize model shorthand if present
    let current_prov = obj.get("default_provider").and_then(|p| p.as_str()).map(|s| s.to_string());
    let mut resolved_provider_to_set = None;
    if let Some(model_val) = obj.get_mut("default_model") {
        if let Some(m) = model_val.as_str().map(|s| s.to_string()) {
            let (resolved_prov, resolved_model) = Config::resolve_model(&m, current_prov.as_deref());
            if resolved_model != m {
                *model_val = Value::String(resolved_model.clone());
                changes.push(format!("Resolved model shorthand '{}' to canonical '{}'", m, resolved_model));
            }
            if current_prov.is_none() || current_prov.as_deref() == Some("deepseek") && resolved_prov != "deepseek" {
                resolved_provider_to_set = Some(resolved_prov);
            }
        }
    }
    if let Some(new_prov) = resolved_provider_to_set {
        changes.push(format!("Auto-detected provider '{}'", new_prov));
        obj.insert("default_provider".to_string(), Value::String(new_prov));
    }

    // 3. Normalize Base URLs
    let url_fields = [
        "openai_base_url",
        "anthropic_base_url",
        "deepseek_base_url",
        "xai_base_url",
        "openrouter_base_url",
        "ollama_base_url",
    ];

    for field in &url_fields {
        if let Some(v) = obj.get_mut(*field) {
            if let Some(s) = v.as_str().map(|s| s.to_string()) {
                if let Some(normalized) = crate::config::sanitize_base_url(&s) {
                    if normalized != s {
                        *v = Value::String(normalized.clone());
                        changes.push(format!("Normalized base URL '{}' in field '{}' to '{}'", s, field, normalized));
                    }
                } else if s.trim().is_empty() {
                    // Empty string URL: remove field to use defaults
                    *v = Value::Null;
                    changes.push(format!("Cleared empty base URL in field '{}'", field));
                }
            }
        }
    }

    // Sane Ollama default URL if missing or null
    match obj.get("ollama_base_url") {
        None | Some(Value::Null) => {
            obj.insert("ollama_base_url".to_string(), Value::String("http://localhost:11434".to_string()));
            changes.push("Configured default Ollama URL 'http://localhost:11434'".to_string());
        }
        _ => {}
    }

    // 4. Sanitize API Keys
    let key_fields = [
        "openai_api_key",
        "anthropic_api_key",
        "deepseek_api_key",
        "xai_api_key",
        "openrouter_api_key",
    ];

    for field in &key_fields {
        if let Some(v) = obj.get_mut(*field) {
            if let Some(s) = v.as_str().map(|s| s.to_string()) {
                if let Some(sanitized) = crate::config::sanitize_env_var(&s) {
                    if sanitized != s {
                        *v = Value::String(sanitized.clone());
                        changes.push(format!("Sanitized API key in field '{}'", field));
                    }
                } else {
                    // Empty or placeholder key: remove or set null
                    *v = Value::Null;
                    changes.push(format!("Removed invalid placeholder API key in field '{}'", field));
                }
            }
        }
    }

    // Clean null fields so serde doesn't serialize empty nulls unnecessarily
    let null_keys: Vec<String> = obj.iter()
        .filter(|(_, v)| v.is_null())
        .map(|(k, _)| k.clone())
        .collect();
    for k in null_keys {
        obj.remove(&k);
    }

    // 5. Temperature clamp validation
    if let Some(temp_val) = obj.get_mut("default_temperature") {
        if let Some(t) = temp_val.as_f64() {
            if t < 0.0 {
                *temp_val = Value::from(0.0);
                changes.push(format!("Clamped negative temperature {} to 0.0", t));
            } else if t > 2.0 {
                *temp_val = Value::from(2.0);
                changes.push(format!("Clamped excessive temperature {} to 2.0", t));
            }
        }
    }

    // 6. Set version to 2
    obj.insert("version".to_string(), Value::from(CURRENT_SCHEMA_VERSION));
    changes.push("Upgraded schema from v1 to v2".to_string());

    Ok(())
}

/// Applies all required migration steps sequentially to an in-memory JSON Value.
///
/// Returns a `MigrationOutcome` detailing the performed migrations.
pub fn migrate_value(val: &mut Value) -> Result<MigrationOutcome, MigrationError> {
    let from_version = detect_version(val);

    if from_version > CURRENT_SCHEMA_VERSION {
        return Err(MigrationError::FutureVersion {
            version: from_version,
            max_supported: CURRENT_SCHEMA_VERSION,
        });
    }

    if from_version == CURRENT_SCHEMA_VERSION {
        return Ok(MigrationOutcome {
            from_version,
            to_version: CURRENT_SCHEMA_VERSION,
            performed_migration: false,
            backup_path: None,
            changes: Vec::new(),
        });
    }

    let mut current_version = from_version;
    let mut changes = Vec::new();

    // Stepwise migration chain
    if current_version == LEGACY_SCHEMA_VERSION {
        migrate_v0_to_v1(val, &mut changes)?;
        current_version = V1_SCHEMA_VERSION;
    }

    if current_version == V1_SCHEMA_VERSION {
        migrate_v1_to_v2(val, &mut changes)?;
        current_version = CURRENT_SCHEMA_VERSION;
    }

    Ok(MigrationOutcome {
        from_version,
        to_version: current_version,
        performed_migration: true,
        backup_path: None,
        changes,
    })
}

/// Migrates a JSON string to the current schema and deserializes it into `Config`.
pub fn migrate_str(json_str: &str) -> Result<(Config, MigrationOutcome), MigrationError> {
    let mut val: Value = serde_json::from_str(json_str)?;
    let outcome = migrate_value(&mut val)?;
    let config: Config = serde_json::from_value(val)?;
    Ok((config, outcome))
}

/// Performs a dry-run migration preview without writing anything to disk.
pub fn preview_migration(json_str: &str) -> Result<(Value, MigrationOutcome), MigrationError> {
    let mut val: Value = serde_json::from_str(json_str)?;
    let outcome = migrate_value(&mut val)?;
    Ok((val, outcome))
}

/// Migrates a configuration file on disk to the latest schema version.
///
/// When `backup` is true and changes are made, creates a backup file (`config.json.v{from_version}.bak`).
pub fn migrate_file(path: &Path, backup: bool) -> Result<(Config, MigrationOutcome), MigrationError> {
    let content = std::fs::read_to_string(path)?;
    let mut val: Value = serde_json::from_str(&content)?;
    let mut outcome = migrate_value(&mut val)?;

    if outcome.performed_migration {
        // Create backup if requested
        if backup {
            let backup_file_name = format!(
                "{}.v{}.bak",
                path.file_name().and_then(|n| n.to_str()).unwrap_or("config.json"),
                outcome.from_version
            );
            let backup_path = path.with_file_name(backup_file_name);
            std::fs::write(&backup_path, &content)?;
            outcome.backup_path = Some(backup_path);
        }

        // Write migrated JSON back to disk
        let pretty = serde_json::to_string_pretty(&val)?;
        std::fs::write(path, pretty)?;
    }

    let config: Config = serde_json::from_value(val)?;
    Ok((config, outcome))
}

/// Automatically loads a configuration file, migrating it to the latest schema if needed.
///
/// If the file does not exist, returns default configuration.
pub fn migrate_file_if_needed(path: &Path) -> Result<Config, MigrationError> {
    if !path.exists() {
        return Ok(Config::default());
    }
    let (cfg, _) = migrate_file(path, true)?;
    Ok(cfg)
}

/// Restores a configuration file from a previously generated backup.
pub fn restore_backup(backup_path: &Path, target_path: &Path) -> Result<(), MigrationError> {
    std::fs::copy(backup_path, target_path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_version() {
        assert_eq!(detect_version(&serde_json::json!({})), 0);
        assert_eq!(detect_version(&serde_json::json!({"version": 1})), 1);
        assert_eq!(detect_version(&serde_json::json!({"version": 2})), 2);
        assert_eq!(detect_version(&serde_json::json!({"schema_version": 1})), 1);
        assert_eq!(detect_version(&serde_json::json!({"version": "invalid"})), 0);
    }

    #[test]
    fn test_migrate_v0_legacy_aliases() {
        let raw = serde_json::json!({
            "provider": "openai",
            "model": "gpt-4o",
            "temperature": "0.7",
            "tokens": "4096",
            "openai_key": "sk-test-key",
            "advisors": "true"
        });

        let mut val = raw;
        let outcome = migrate_value(&mut val).expect("migration succeeds");

        assert_eq!(outcome.from_version, 0);
        assert_eq!(outcome.to_version, CURRENT_SCHEMA_VERSION);
        assert!(outcome.performed_migration);
        assert!(!outcome.changes.is_empty());

        let obj = val.as_object().unwrap();
        assert_eq!(obj.get("version").unwrap().as_u64(), Some(2));
        assert_eq!(obj.get("default_provider").unwrap().as_str(), Some("openai"));
        assert_eq!(obj.get("default_model").unwrap().as_str(), Some("gpt-4o"));
        assert_eq!(obj.get("default_temperature").unwrap().as_f64(), Some(0.7));
        assert_eq!(obj.get("max_tokens").unwrap().as_u64(), Some(4096));
        assert_eq!(obj.get("openai_api_key").unwrap().as_str(), Some("sk-test-key"));
        assert_eq!(obj.get("advisors_enabled").unwrap().as_bool(), Some(true));

        // Old keys must be cleaned up
        assert!(!obj.contains_key("provider"));
        assert!(!obj.contains_key("model"));
        assert!(!obj.contains_key("tokens"));
        assert!(!obj.contains_key("openai_key"));
        assert!(!obj.contains_key("advisors"));
    }

    #[test]
    fn test_migrate_v0_generic_keys() {
        let raw = serde_json::json!({
            "provider": "anthropic",
            "model": "claude-3-5-sonnet-20241022",
            "api_key": "sk-ant-test",
            "base_url": "https://custom.anthropic.proxy/v1"
        });

        let mut val = raw;
        let outcome = migrate_value(&mut val).expect("migration succeeds");
        assert!(outcome.performed_migration);

        let obj = val.as_object().unwrap();
        assert_eq!(obj.get("anthropic_api_key").unwrap().as_str(), Some("sk-ant-test"));
        assert_eq!(obj.get("anthropic_base_url").unwrap().as_str(), Some("https://custom.anthropic.proxy/v1"));
        assert!(!obj.contains_key("api_key"));
        assert!(!obj.contains_key("base_url"));
    }

    #[test]
    fn test_migrate_v1_to_v2_sanitization_and_normalization() {
        let raw = serde_json::json!({
            "version": 1,
            "default_provider": "DEEPSEEK",
            "default_model": "r1",
            "deepseek_base_url": "https://api.deepseek.com/v1/",
            "deepseek_api_key": "  sk-ds-key-123  ",
            "openai_api_key": "YOUR_API_KEY",
            "default_temperature": 2.5,
            "advisors_enabled": true
        });

        let mut val = raw;
        let outcome = migrate_value(&mut val).expect("migration succeeds");

        assert_eq!(outcome.from_version, 1);
        assert_eq!(outcome.to_version, 2);
        assert!(outcome.performed_migration);

        let obj = val.as_object().unwrap();
        assert_eq!(obj.get("version").unwrap().as_u64(), Some(2));
        assert_eq!(obj.get("default_provider").unwrap().as_str(), Some("deepseek"));
        // "r1" resolved to "deepseek-reasoner"
        assert_eq!(obj.get("default_model").unwrap().as_str(), Some("deepseek-reasoner"));
        // Base URL trailing slash removed
        assert_eq!(obj.get("deepseek_base_url").unwrap().as_str(), Some("https://api.deepseek.com/v1"));
        // API key trimmed
        assert_eq!(obj.get("deepseek_api_key").unwrap().as_str(), Some("sk-ds-key-123"));
        // Placeholder openai key removed
        assert!(!obj.contains_key("openai_api_key"));
        // Temperature clamped to 2.0
        assert_eq!(obj.get("default_temperature").unwrap().as_f64(), Some(2.0));
        // Default ollama url added
        assert_eq!(obj.get("ollama_base_url").unwrap().as_str(), Some("http://localhost:11434"));
    }

    #[test]
    fn test_migrate_already_current_version() {
        let raw = serde_json::json!({
            "version": 2,
            "default_provider": "deepseek",
            "default_model": "deepseek-chat",
            "advisors_enabled": true
        });

        let mut val = raw;
        let outcome = migrate_value(&mut val).expect("migration succeeds");

        assert_eq!(outcome.from_version, 2);
        assert_eq!(outcome.to_version, 2);
        assert!(!outcome.performed_migration);
        assert!(outcome.changes.is_empty());
    }

    #[test]
    fn test_future_version_error() {
        let mut raw = serde_json::json!({
            "version": 999,
            "default_provider": "quantum-llm"
        });

        let err = migrate_value(&mut raw).expect_err("future version should fail");
        match err {
            MigrationError::FutureVersion { version, max_supported } => {
                assert_eq!(version, 999);
                assert_eq!(max_supported, CURRENT_SCHEMA_VERSION);
            }
            _ => panic!("Expected FutureVersion error"),
        }
    }

    #[test]
    fn test_migrate_str_to_config() {
        let json = r#"{
            "provider": "deepseek",
            "model": "deepseek-chat",
            "temperature": 0.5,
            "advisors": true
        }"#;

        let (cfg, outcome) = migrate_str(json).expect("migrate str succeeds");
        assert_eq!(cfg.version, CURRENT_SCHEMA_VERSION);
        assert_eq!(cfg.default_provider, "deepseek");
        assert_eq!(cfg.default_model, "deepseek-chat");
        assert_eq!(cfg.default_temperature, Some(0.5));
        assert!(cfg.advisors_enabled);
        assert!(outcome.performed_migration);
    }

    #[test]
    fn test_migrate_file_with_backup_and_restore() {
        let tmp_dir = std::env::temp_dir().join(format!("fusion_mig_test_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp_dir).unwrap();
        let config_file = tmp_dir.join("config.json");

        let legacy_content = r#"{
            "provider": "openrouter",
            "model": "meta-llama/llama-3-70b",
            "openrouter_key": "sk-or-test"
        }"#;
        std::fs::write(&config_file, legacy_content).unwrap();

        // Perform file migration with backup
        let (cfg, outcome) = migrate_file(&config_file, true).expect("migrate file succeeds");
        assert_eq!(cfg.version, CURRENT_SCHEMA_VERSION);
        assert_eq!(cfg.default_provider, "openrouter");
        assert_eq!(cfg.default_model, "meta-llama/llama-3-70b");
        assert_eq!(cfg.openrouter_api_key.as_deref(), Some("sk-or-test"));
        assert!(outcome.performed_migration);

        // Verify backup was created
        let backup_path = outcome.backup_path.expect("backup path exists");
        assert!(backup_path.exists());
        let backup_content = std::fs::read_to_string(&backup_path).unwrap();
        assert_eq!(backup_content, legacy_content);

        // Verify updated file has version: 2
        let updated_content = std::fs::read_to_string(&config_file).unwrap();
        assert!(updated_content.contains("\"version\": 2"));
        assert!(updated_content.contains("\"default_provider\": \"openrouter\""));

        // Test restore
        restore_backup(&backup_path, &config_file).expect("restore backup succeeds");
        let restored_content = std::fs::read_to_string(&config_file).unwrap();
        assert_eq!(restored_content, legacy_content);

        // Cleanup
        let _ = std::fs::remove_dir_all(&tmp_dir);
    }
}
