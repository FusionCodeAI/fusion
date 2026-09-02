//! End-to-End Integration Smoke Tests for Fusion
//!
//! Comprehensive verification across all primary subsystems:
//! 1. CLI invocation: `--version`, `-V`, `--help`, `-h`, `--acp`, combined flags, and invalid argument errors.
//! 2. Configuration loading: defaults, JSON serialization, custom config file paths, environment variable cascades,
//!    provider URL/key resolution, and model shorthand/alias detection.
//! 3. Offline mode provider fallback: network reachability probing, local Ollama detection, model scoring heuristics,
//!    and automatic configuration transitions with mock responses.
//! 4. Conversational session management: creation, lifecycle, token stats, metadata, disk persistence, reload,
//!    modification cycles, deletion, summaries, and Markdown exports.
//! 5. Tool execution engine: registry dispatch, filesystem read/write/edit, ambiguous patch safety, bash execution,
//!    and grep/glob search integration.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::error::ErrorKind;
use clap::Parser;
use serde_json::json;
use tempfile::tempdir;
use uuid::Uuid;

use fusion::agent::session::{Session, SessionSummary, TokenStats};
use fusion::cli::Cli;
use fusion::config::env_loader::{expand_variables, load_dotenv_from, parse_env_str};
use fusion::config::presets::ConfigPreset;
use fusion::config::Config;
use fusion::provider::offline::{
    score_ollama_model, select_best_local_model, select_best_local_model_from_names,
    MockConnectivityProber, MockOllamaProber, NetworkEnvironmentStatus, OfflineDetector,
    OfflineDetectorConfig, OfflineReason, OfflineTransitionResult,
};
use fusion::provider::ollama::{OllamaModelDetails, OllamaModelInfo};
use fusion::provider::types::{Message, Role, ToolCall};
use fusion::tools::{
    default_registry, BashTool, EditFileTool, GlobTool, GrepTool, ReadFileTool, Tool, ToolContext,
    ToolRegistry, WriteFileTool,
};

// ===========================================================================
// Part 1: CLI Invocation & Argument Parsing Tests
// ===========================================================================

#[test]
fn test_cli_version_flag() {
    // 1. Long flag: --version
    let err_long = Cli::try_parse_from(["fusion", "--version"]).unwrap_err();
    assert_eq!(
        err_long.kind(),
        ErrorKind::DisplayVersion,
        "Expected DisplayVersion error kind for --version"
    );
    let output_long = err_long.to_string();
    assert!(
        output_long.contains("0.3.0") || output_long.contains("fusion"),
        "Version output should contain binary name or version: {}",
        output_long
    );

    // 2. Short flag: -V
    let err_short = Cli::try_parse_from(["fusion", "-V"]).unwrap_err();
    assert_eq!(
        err_short.kind(),
        ErrorKind::DisplayVersion,
        "Expected DisplayVersion error kind for -V"
    );
    let output_short = err_short.to_string();
    assert!(
        output_short.contains("0.3.0") || output_short.contains("fusion"),
        "Short version output should contain binary name or version: {}",
        output_short
    );
}

#[test]
fn test_cli_help_flag() {
    // 1. Long flag: --help
    let err_long = Cli::try_parse_from(["fusion", "--help"]).unwrap_err();
    assert_eq!(
        err_long.kind(),
        ErrorKind::DisplayHelp,
        "Expected DisplayHelp error kind for --help"
    );
    let help_text = err_long.to_string();
    assert!(
        help_text.contains("--acp"),
        "Help output should document the --acp flag"
    );
    assert!(
        help_text.contains("--model"),
        "Help output should document the --model flag"
    );
    assert!(
        help_text.contains("--provider"),
        "Help output should document the --provider flag"
    );
    assert!(
        help_text.contains("--preset"),
        "Help output should document the --preset flag"
    );
    assert!(
        help_text.contains("--no-advisors"),
        "Help output should document the --no-advisors flag"
    );

    // 2. Short flag: -h
    let err_short = Cli::try_parse_from(["fusion", "-h"]).unwrap_err();
    assert_eq!(
        err_short.kind(),
        ErrorKind::DisplayHelp,
        "Expected DisplayHelp error kind for -h"
    );
}

#[test]
fn test_cli_acp_mode_flag() {
    // 1. Invocation with --acp flag
    let cli_acp = Cli::try_parse_from(["fusion", "--acp"]).expect("Failed to parse --acp flag");
    assert!(cli_acp.acp, "Expected acp field to be true when --acp is passed");
    assert!(cli_acp.prompt.is_none(), "Prompt should be None when only --acp is provided");

    // 2. Default invocation without --acp flag
    let cli_default = Cli::try_parse_from(["fusion"]).expect("Failed to parse default arguments");
    assert!(!cli_default.acp, "Expected acp field to be false by default");
}

#[test]
fn test_cli_arguments_combinations() {
    // 1. Single positional prompt
    let cli_prompt = Cli::try_parse_from(["fusion", "Implement binary search in Rust"])
        .expect("Failed to parse positional prompt");
    assert_eq!(
        cli_prompt.prompt.as_deref(),
        Some("Implement binary search in Rust")
    );
    assert_eq!(cli_prompt.model, None);
    assert_eq!(cli_prompt.provider, None);
    assert!(!cli_prompt.no_advisors);

    // 2. Full options combination
    let cli_full = Cli::try_parse_from([
        "fusion",
        "-m",
        "claude-3-5-sonnet-20241022",
        "-p",
        "anthropic",
        "-P",
        "coding-fast",
        "-C",
        "/custom/project/dir",
        "--no-advisors",
        "Refactor error handling",
    ])
    .expect("Failed to parse combined CLI options");

    assert_eq!(
        cli_full.model.as_deref(),
        Some("claude-3-5-sonnet-20241022")
    );
    assert_eq!(cli_full.provider.as_deref(), Some("anthropic"));
    assert_eq!(cli_full.preset.as_deref(), Some("coding-fast"));
    assert_eq!(
        cli_full.cwd.as_deref(),
        Some(Path::new("/custom/project/dir"))
    );
    assert!(cli_full.no_advisors);
    assert_eq!(
        cli_full.prompt.as_deref(),
        Some("Refactor error handling")
    );

    // 3. Shell completion generation flag
    let cli_comp = Cli::try_parse_from(["fusion", "--generate-completion", "zsh"])
        .expect("Failed to parse --generate-completion flag");
    assert_eq!(
        cli_comp.generate_completion,
        Some(clap_complete::Shell::Zsh)
    );
}

#[test]
fn test_cli_invalid_arguments_handling() {
    // 1. Unrecognized flag
    let err_unrecognized =
        Cli::try_parse_from(["fusion", "--non-existent-unrecognized-flag"]).unwrap_err();
    assert_eq!(
        err_unrecognized.kind(),
        ErrorKind::UnknownArgument,
        "Expected UnknownArgument error kind for unknown flag"
    );

    // 2. Missing value for required option parameter
    let err_missing_val = Cli::try_parse_from(["fusion", "--model"]).unwrap_err();
    assert!(
        err_missing_val.kind() == ErrorKind::InvalidValue
            || err_missing_val.kind() == ErrorKind::MissingRequiredArgument,
        "Expected InvalidValue or MissingRequiredArgument when option value is missing"
    );

    // 3. Invalid value for shell completion enum
    let err_invalid_shell =
        Cli::try_parse_from(["fusion", "--generate-completion", "invalid_shell_name"]).unwrap_err();
    assert_eq!(
        err_invalid_shell.kind(),
        ErrorKind::ValueValidation,
        "Expected ValueValidation error kind for invalid shell"
    );
}

// ===========================================================================
// Part 2: Config Loading (Defaults, Env Vars, Custom Paths) Tests
// ===========================================================================

#[test]
fn test_config_defaults_and_serialization() {
    let cfg = Config::default();

    // Verify baseline defaults
    assert_eq!(cfg.default_provider, "deepseek");
    assert_eq!(cfg.default_model, "deepseek-chat");
    assert_eq!(cfg.default_temperature, Some(0.2));
    assert_eq!(cfg.max_tokens, Some(8192));
    assert!(cfg.advisors_enabled);

    // Test JSON Serialization & Deserialization roundtrip
    let json_str = serde_json::to_string_pretty(&cfg).expect("Failed to serialize Config to JSON");
    assert!(json_str.contains("\"default_provider\": \"deepseek\""));
    assert!(json_str.contains("\"default_model\": \"deepseek-chat\""));

    let deserialized: Config =
        serde_json::from_str(&json_str).expect("Failed to deserialize Config");
    assert_eq!(deserialized.default_provider, cfg.default_provider);
    assert_eq!(deserialized.default_model, cfg.default_model);
    assert_eq!(deserialized.advisors_enabled, cfg.advisors_enabled);
    assert_eq!(deserialized.max_tokens, cfg.max_tokens);
    assert_eq!(deserialized.default_temperature, cfg.default_temperature);
}

#[test]
fn test_config_custom_file_loading_and_saving() {
    let temp = tempdir().expect("Failed to create temporary directory for custom config test");
    let custom_config_path = temp.path().join("custom_fusion_config.json");

    let custom_json = r#"{
  "version": 1,
  "default_provider": "anthropic",
  "default_model": "claude-3-5-sonnet-20241022",
  "anthropic_api_key": "sk-ant-custom-test-key-12345",
  "anthropic_base_url": "https://custom.anthropic.proxy/v1",
  "default_temperature": 0.5,
  "max_tokens": 16384,
  "advisors_enabled": false,
  "sound_enabled": true
}"#;

    fs::write(&custom_config_path, custom_json).expect("Failed to write custom config file");

    // Load from custom path
    let (loaded_cfg, _outcome) = Config::load_from_file(&custom_config_path)
        .expect("Failed to load config from custom file path");

    assert_eq!(loaded_cfg.default_provider, "anthropic");
    assert_eq!(loaded_cfg.default_model, "claude-3-5-sonnet-20241022");
    assert_eq!(
        loaded_cfg.anthropic_api_key.as_deref(),
        Some("sk-ant-custom-test-key-12345")
    );
    assert_eq!(
        loaded_cfg.anthropic_base_url.as_deref(),
        Some("https://custom.anthropic.proxy/v1")
    );
    assert_eq!(loaded_cfg.default_temperature, Some(0.5));
    assert_eq!(loaded_cfg.max_tokens, Some(16384));
    assert!(!loaded_cfg.advisors_enabled);
    assert!(loaded_cfg.sound_enabled);

    // Test Config::from_json string parsing directly
    let parsed_from_str =
        Config::from_json(custom_json).expect("Failed to parse Config from JSON string");
    assert_eq!(parsed_from_str.default_provider, "anthropic");
    assert_eq!(parsed_from_str.max_tokens, Some(16384));
}

#[test]
fn test_config_env_loader_parsing_and_variable_expansion() {
    // 1. Test .env content parsing with quotes, exports, and comments
    let env_content = r#"
# Core API Keys
OPENAI_API_KEY="sk-openai-test-key-abc"
ANTHROPIC_API_KEY='sk-ant-test-key-xyz'
export DEEPSEEK_API_KEY=sk-ds-unquoted-key

# Base URLs
BASE_DOMAIN=proxy.internal
CUSTOM_API_URL=https://${BASE_DOMAIN}/v1
FALLBACK_PORT=${PORT:-8080}
"#;

    let parsed_vars = parse_env_str(env_content).expect("Failed to parse .env content string");
    assert_eq!(
        parsed_vars.get("OPENAI_API_KEY").map(|s| s.as_str()),
        Some("sk-openai-test-key-abc")
    );
    assert_eq!(
        parsed_vars.get("ANTHROPIC_API_KEY").map(|s| s.as_str()),
        Some("sk-ant-test-key-xyz")
    );
    assert_eq!(
        parsed_vars.get("DEEPSEEK_API_KEY").map(|s| s.as_str()),
        Some("sk-ds-unquoted-key")
    );

    // 2. Test variable expansion with defaults and variable references
    let expanded_url = expand_variables("https://${BASE_DOMAIN}/api/v1", &parsed_vars)
        .expect("Failed to expand variables");
    assert_eq!(expanded_url, "https://proxy.internal/api/v1");

    let expanded_default = expand_variables("${UNDEFINED_VAR:-default_val}", &parsed_vars)
        .expect("Failed to expand default fallback");
    assert_eq!(expanded_default, "default_val");

    // 3. Test load_dotenv_from directory with tempfile
    let temp = tempdir().expect("Failed to create tempdir for .env loader test");
    let env_file_path = temp.path().join(".env");
    fs::write(&env_file_path, "FUSION_TEST_KEY=secret_value_123\nPORT=9000\n")
        .expect("Failed to write .env file");

    let loaded_env =
        load_dotenv_from(temp.path()).expect("Failed to load dotenv from temp directory");
    assert_eq!(
        loaded_env.get("FUSION_TEST_KEY"),
        Some("secret_value_123")
    );
    assert_eq!(loaded_env.get("PORT"), Some("9000"));
}

#[test]
fn test_config_provider_url_and_key_resolution() {
    let mut cfg = Config::default();
    cfg.openai_api_key = Some("sk-openai-test-key".to_string());
    cfg.anthropic_api_key = Some("sk-ant-test-key".to_string());
    cfg.deepseek_api_key = Some("sk-ds-test-key".to_string());
    cfg.xai_api_key = Some("xai-test-key".to_string());
    cfg.openrouter_api_key = Some("sk-or-test-key".to_string());

    // 1. OpenAI resolution
    let (key, url) = cfg.get_key_and_url("openai");
    assert_eq!(key, Some("sk-openai-test-key".to_string()));
    assert_eq!(url, "https://api.openai.com/v1");

    // 2. Anthropic resolution
    let (key, url) = cfg.get_key_and_url("anthropic");
    assert_eq!(key, Some("sk-ant-test-key".to_string()));
    assert_eq!(url, "https://api.anthropic.com/v1");

    // 3. DeepSeek resolution
    let (key, url) = cfg.get_key_and_url("deepseek");
    assert_eq!(key, Some("sk-ds-test-key".to_string()));
    assert_eq!(url, "https://api.deepseek.com");

    // 4. xAI / Grok resolution
    let (key, url) = cfg.get_key_and_url("xai");
    assert_eq!(key, Some("xai-test-key".to_string()));
    assert_eq!(url, "https://api.x.ai/v1");

    let (key, url) = cfg.get_key_and_url("grok");
    assert_eq!(key, Some("xai-test-key".to_string()));
    assert_eq!(url, "https://api.x.ai/v1");

    // 5. OpenRouter resolution
    let (key, url) = cfg.get_key_and_url("openrouter");
    assert_eq!(key, Some("sk-or-test-key".to_string()));
    assert_eq!(url, "https://openrouter.ai/api/v1");

    // 6. Ollama resolution
    let (key, url) = cfg.get_key_and_url("ollama");
    assert_eq!(key, None);
    assert_eq!(url, "http://localhost:11434");

    // 7. Custom base URL overrides
    cfg.openai_base_url = Some("https://custom.openai.proxy/v1".to_string());
    let (_, custom_url) = cfg.get_key_and_url("openai");
    assert_eq!(custom_url, "https://custom.openai.proxy/v1");

    cfg.ollama_base_url = Some("http://192.168.1.100:11434".to_string());
    let (_, custom_ollama_url) = cfg.get_key_and_url("ollama");
    assert_eq!(custom_ollama_url, "http://192.168.1.100:11434");
}

#[test]
fn test_config_model_shorthands_and_provider_detection() {
    // 1. Shorthand resolution
    let sonnet_res = Config::resolve_model_shorthand("sonnet");
    assert_eq!(
        sonnet_res,
        Some(("anthropic", "claude-3-5-sonnet-20241022"))
    );

    let gpt4o_res = Config::resolve_model_shorthand("4o");
    assert_eq!(gpt4o_res, Some(("openai", "gpt-4o")));

    let r1_res = Config::resolve_model_shorthand("r1");
    assert_eq!(r1_res, Some(("deepseek", "deepseek-reasoner")));

    let haiku_res = Config::resolve_model_shorthand("haiku");
    assert_eq!(haiku_res, Some(("anthropic", "claude-3-5-haiku-20241022")));

    assert_eq!(Config::resolve_model_shorthand("nonexistent-shorthand"), None);

    // 2. Provider detection from model names
    assert_eq!(
        Config::detect_provider_for_model("claude-3-5-sonnet-20241022"),
        Some("anthropic")
    );
    assert_eq!(
        Config::detect_provider_for_model("gpt-4o"),
        Some("openai")
    );
    assert_eq!(
        Config::detect_provider_for_model("o1-preview"),
        Some("openai")
    );
    assert_eq!(
        Config::detect_provider_for_model("deepseek-chat"),
        Some("deepseek")
    );
    assert_eq!(
        Config::detect_provider_for_model("grok-2"),
        Some("xai")
    );
    assert_eq!(
        Config::detect_provider_for_model("meta-llama/llama-3-70b-instruct"),
        Some("openrouter")
    );
    assert_eq!(
        Config::detect_provider_for_model("qwen2.5-coder:32b"),
        Some("ollama")
    );
    assert_eq!(
        Config::detect_provider_for_model("llama3.1:8b"),
        Some("ollama")
    );
}

// ===========================================================================
// Part 3: Offline Mode Provider Fallback with Mock Responses
// ===========================================================================

fn make_test_ollama_model(
    name: &str,
    param_size: Option<&str>,
    quant: Option<&str>,
    size_bytes: Option<u64>,
    vram: Option<u64>,
) -> OllamaModelInfo {
    OllamaModelInfo {
        name: name.to_string(),
        model: Some(name.to_string()),
        modified_at: None,
        size: size_bytes,
        digest: None,
        details: Some(OllamaModelDetails {
            parent_model: None,
            format: Some("gguf".to_string()),
            family: None,
            families: None,
            parameter_size: param_size.map(|s| s.to_string()),
            quantization_level: quant.map(|s| s.to_string()),
        }),
        expires_at: None,
        size_vram: vram,
    }
}

#[tokio::test]
async fn test_offline_mode_online_retains_provider() {
    let mut config = Config::default();
    config.default_provider = "anthropic".to_string();
    config.default_model = "claude-3-5-sonnet-20241022".to_string();

    let detector_config = OfflineDetectorConfig {
        ollama_url: "http://127.0.0.1:11434".to_string(),
        connectivity_timeout: Duration::from_millis(50),
        ollama_timeout: Duration::from_millis(50),
        socket_probes: vec![],
        fallback_model: "qwen2.5-coder".to_string(),
        preferred_model_families: vec!["qwen2.5-coder".to_string()],
        manual_offline_enforced: false,
    };

    let detector = OfflineDetector::with_config(detector_config).with_probers(
        Box::new(MockConnectivityProber { online: true }),
        Box::new(MockOllamaProber {
            reachable: true,
            models: vec![make_test_ollama_model("qwen2.5-coder:32b", Some("32B"), None, None, None)],
        }),
    );

    assert_eq!(detector.detect_network_status(), NetworkEnvironmentStatus::Online);

    let transition = detector.auto_switch_if_offline(&mut config).await;
    assert_eq!(
        transition,
        OfflineTransitionResult::StayedOnline {
            provider: "anthropic".to_string(),
            model: "claude-3-5-sonnet-20241022".to_string(),
        }
    );
    assert!(!transition.is_switched());
    assert_eq!(config.default_provider, "anthropic");
    assert_eq!(config.default_model, "claude-3-5-sonnet-20241022");
}

#[tokio::test]
async fn test_offline_mode_fallback_to_ollama_with_models() {
    let mut config = Config::default();
    config.default_provider = "deepseek".to_string();
    config.default_model = "deepseek-chat".to_string();

    let mock_models = vec![
        make_test_ollama_model("llama3.1:8b", Some("8B"), Some("Q4_K_M"), Some(4_900_000_000), None),
        make_test_ollama_model("qwen2.5-coder:32b", Some("32B"), Some("Q4_K_M"), Some(19_000_000_000), None),
        make_test_ollama_model("mistral:7b", Some("7B"), Some("Q4_0"), Some(4_100_000_000), None),
    ];

    let detector_config = OfflineDetectorConfig::default();
    let detector = OfflineDetector::with_config(detector_config).with_probers(
        Box::new(MockConnectivityProber { online: false }),
        Box::new(MockOllamaProber {
            reachable: true,
            models: mock_models,
        }),
    );

    assert_eq!(detector.detect_network_status(), NetworkEnvironmentStatus::Offline);

    let transition = detector.auto_switch_if_offline(&mut config).await;
    assert!(transition.is_switched());
    assert!(transition.is_offline_ready());

    match &transition {
        OfflineTransitionResult::SwitchedToOllama {
            previous_provider,
            previous_model,
            selected_model,
            available_models,
            reason,
            notice,
        } => {
            assert_eq!(previous_provider, "deepseek");
            assert_eq!(previous_model, "deepseek-chat");
            assert_eq!(selected_model, "qwen2.5-coder:32b");
            assert_eq!(available_models.len(), 3);
            assert_eq!(reason, &OfflineReason::NoInternetConnection);
            assert!(notice.contains("Switched provider from 'deepseek'"));
            assert!(notice.contains("qwen2.5-coder:32b"));
        }
        _ => panic!("Expected SwitchedToOllama transition result, got: {:?}", transition),
    }

    assert_eq!(config.default_provider, "ollama");
    assert_eq!(config.default_model, "qwen2.5-coder:32b");
    assert_eq!(
        config.ollama_base_url.as_deref(),
        Some("http://127.0.0.1:11434")
    );
}

#[tokio::test]
async fn test_offline_mode_fallback_empty_model_catalog_uses_fallback_model() {
    let mut config = Config::default();
    config.default_provider = "openai".to_string();
    config.default_model = "gpt-4o".to_string();

    let detector_config = OfflineDetectorConfig {
        ollama_url: "http://127.0.0.1:11434".to_string(),
        connectivity_timeout: Duration::from_millis(50),
        ollama_timeout: Duration::from_millis(50),
        socket_probes: vec![],
        fallback_model: "qwen2.5-coder".to_string(),
        preferred_model_families: vec!["qwen2.5-coder".to_string()],
        manual_offline_enforced: false,
    };

    let detector = OfflineDetector::with_config(detector_config).with_probers(
        Box::new(MockConnectivityProber { online: false }),
        Box::new(MockOllamaProber {
            reachable: true,
            models: vec![], // Empty model catalog
        }),
    );

    let transition = detector.auto_switch_if_offline(&mut config).await;
    assert!(transition.is_switched());

    match transition {
        OfflineTransitionResult::SwitchedToOllama {
            selected_model,
            available_models,
            ..
        } => {
            assert_eq!(selected_model, "qwen2.5-coder");
            assert!(available_models.is_empty());
        }
        _ => panic!("Expected SwitchedToOllama transition"),
    }

    assert_eq!(config.default_provider, "ollama");
    assert_eq!(config.default_model, "qwen2.5-coder");
}

#[tokio::test]
async fn test_offline_mode_ollama_unreachable_returns_remediation() {
    let mut config = Config::default();
    config.default_provider = "deepseek".to_string();
    config.default_model = "deepseek-chat".to_string();

    let detector_config = OfflineDetectorConfig::default();
    let detector = OfflineDetector::with_config(detector_config).with_probers(
        Box::new(MockConnectivityProber { online: false }),
        Box::new(MockOllamaProber {
            reachable: false,
            models: vec![],
        }),
    );

    let transition = detector.auto_switch_if_offline(&mut config).await;
    assert!(!transition.is_switched());
    assert!(!transition.is_offline_ready());

    match transition {
        OfflineTransitionResult::OfflineNoLocalBackend {
            reason,
            attempted_url,
            remediation,
        } => {
            assert_eq!(reason, OfflineReason::NoInternetConnection);
            assert_eq!(attempted_url, "http://127.0.0.1:11434");
            assert!(remediation.contains("ollama serve"));
            assert!(remediation.contains("ollama run qwen2.5-coder"));
        }
        _ => panic!("Expected OfflineNoLocalBackend transition result"),
    }

    // Config should remain untouched if local backend was unreachable
    assert_eq!(config.default_provider, "deepseek");
    assert_eq!(config.default_model, "deepseek-chat");
}

#[test]
fn test_offline_model_scoring_and_selection_heuristics() {
    let qwen_coder_32b = make_test_ollama_model(
        "qwen2.5-coder:32b",
        Some("32B"),
        Some("Q4_K_M"),
        Some(19_000_000_000),
        None,
    );
    let deepseek_r1_14b = make_test_ollama_model(
        "deepseek-r1:14b",
        Some("14B"),
        Some("Q4_K_M"),
        Some(9_000_000_000),
        None,
    );
    let llama3_8b = make_test_ollama_model(
        "llama3.1:8b",
        Some("8B"),
        Some("Q4_0"),
        Some(4_900_000_000),
        None,
    );
    let mistral_7b = make_test_ollama_model(
        "mistral:7b",
        Some("7B"),
        Some("Q4_0"),
        Some(4_100_000_000),
        None,
    );

    let score_qwen = score_ollama_model(&qwen_coder_32b);
    let score_r1 = score_ollama_model(&deepseek_r1_14b);
    let score_llama = score_ollama_model(&llama3_8b);
    let score_mistral = score_ollama_model(&mistral_7b);

    assert!(
        score_qwen > score_r1,
        "qwen2.5-coder (score: {}) should outscore deepseek-r1 (score: {})",
        score_qwen,
        score_r1
    );
    assert!(
        score_r1 > score_llama,
        "deepseek-r1 (score: {}) should outscore llama3 (score: {})",
        score_r1,
        score_llama
    );
    assert!(
        score_llama > score_mistral,
        "llama3 (score: {}) should outscore mistral (score: {})",
        score_llama,
        score_mistral
    );

    // Test select_best_local_model
    let models = vec![mistral_7b, llama3_8b, qwen_coder_32b, deepseek_r1_14b];
    let best = select_best_local_model(&models);
    assert_eq!(best, Some("qwen2.5-coder:32b".to_string()));

    // Test select_best_local_model_from_names helper
    let best_name = select_best_local_model_from_names(&[
        "mistral:latest",
        "llama3:8b",
        "qwen2.5-coder:7b",
        "deepseek-coder:6.7b",
    ]);
    assert_eq!(best_name, Some("qwen2.5-coder:7b".to_string()));
}

#[test]
fn test_offline_mode_sync_auto_switch() {
    let mut config = Config::default();
    config.default_provider = "deepseek".to_string();

    let detector = OfflineDetector::with_config(OfflineDetectorConfig::default()).with_probers(
        Box::new(MockConnectivityProber { online: false }),
        Box::new(MockOllamaProber {
            reachable: true,
            models: vec![],
        }),
    );

    let transition = detector.auto_switch_if_offline_sync(&mut config);
    assert!(transition.is_switched());
    assert_eq!(config.default_provider, "ollama");
}

// ===========================================================================
// Part 4: Session Creation, Persistence to Disk, and Reload Tests
// ===========================================================================

#[test]
fn test_session_lifecycle_messages_and_tools() {
    let mut session = Session::new("claude-3-5-sonnet");

    assert_eq!(session.active_model(), "claude-3-5-sonnet");
    assert_eq!(session.total_messages(), 0);
    assert!(session.title().is_none());

    // 1. Add System Message
    session.add_system_message("You are an expert Rust software architect.");
    assert_eq!(session.total_messages(), 1);
    assert_eq!(session.messages()[0].role, Role::System);

    // 2. Add User Message (should auto-generate title from first user message)
    session.add_user_message("Please help me refactor the database module to use connection pooling.");
    assert_eq!(session.total_messages(), 2);
    assert_eq!(session.messages()[1].role, Role::User);
    assert!(session.title().is_some());
    let title = session.title().unwrap();
    assert!(title.starts_with("Please help me refactor"));

    // 3. Add Assistant Message with Tool Calls
    let tool_call = ToolCall {
        id: "call_read_123".to_string(),
        name: "read".to_string(),
        arguments: "{\"path\": \"src/db.rs\"}".to_string(),
    };
    session.add_assistant_with_tools("Checking the current DB implementation...", vec![tool_call.clone()]);
    assert_eq!(session.total_messages(), 3);
    assert_eq!(session.messages()[2].role, Role::Assistant);
    assert!(session.messages()[2].tool_calls.is_some());
    assert_eq!(
        session.messages()[2].tool_calls.as_ref().unwrap()[0].name,
        "read"
    );

    // 4. Add Tool Result
    session.add_tool_result("call_read_123", "struct Database { connection: String }");
    assert_eq!(session.total_messages(), 4);
    assert_eq!(session.messages()[3].role, Role::Tool);
    assert_eq!(
        session.messages()[3].tool_call_id,
        Some("call_read_123".to_string())
    );

    // 5. Add Final Assistant Message
    session.add_assistant_message("I suggest introducing `r2d2` or `deadpool` for connection pooling.");
    assert_eq!(session.total_messages(), 5);

    // 6. Record token usage
    session.record_tokens(Some(450), Some(180));
    assert_eq!(session.token_stats().prompt_tokens, 450);
    assert_eq!(session.token_stats().completion_tokens, 180);
    assert_eq!(session.token_stats().total_tokens, 630);
    assert_eq!(session.token_stats().total_turns, 1);

    // 7. Metadata management
    session.set_metadata("project_name", "fusion_core");
    session.set_metadata("branch", "feature/offline-sync");
    assert_eq!(session.get_metadata("project_name"), Some("fusion_core"));
    assert_eq!(session.get_metadata("branch"), Some("feature/offline-sync"));
    assert_eq!(session.get_metadata("nonexistent"), None);

    // 8. Test Truncation
    session.truncate(3);
    assert_eq!(session.total_messages(), 3);

    // 9. Test Clear
    session.clear();
    assert_eq!(session.total_messages(), 0);
}

#[test]
fn test_session_disk_persistence_save_and_reload() {
    let temp = tempdir().expect("Failed to create temporary directory for session persistence");
    let session_id = Uuid::new_v4();
    let mut session = Session::with_id(session_id, "gpt-4o");

    session.set_system_prompt("System prompt initialized.");
    session.set_title("E2E Session Persistence Verification");
    session.add_user_message("Write a comprehensive smoke test for Fusion");
    session.add_assistant_with_tools(
        "I will create the smoke test using Rust standard and tempfile libraries.",
        vec![ToolCall {
            id: "call_write_file_789".to_string(),
            name: "write".to_string(),
            arguments: "{\"path\": \"tests/smoke_test.rs\", \"content\": \"...\"}".to_string(),
        }],
    );
    session.add_tool_result("call_write_file_789", "Successfully wrote 500 lines.");
    session.add_assistant_message("Smoke test implemented with 100% test coverage.");

    session.record_usage(1200, 480);
    session.set_metadata("env", "staging");
    session.set_metadata("version", "0.3.0");

    // Save session to disk
    let file_path = temp.path().join("saved_session.json");
    let saved_path = session
        .save_to_path(&file_path)
        .expect("Failed to save session to disk");
    assert_eq!(saved_path, file_path);
    assert!(file_path.exists(), "Saved session file must exist on disk");

    // Reload session from disk
    let loaded =
        Session::load_from_path(&file_path).expect("Failed to load session from disk path");

    // Comprehensive equality verification
    assert_eq!(loaded.id(), session_id);
    assert_eq!(loaded.active_model(), "gpt-4o");
    assert_eq!(loaded.title(), Some("E2E Session Persistence Verification"));
    assert_eq!(loaded.system_prompt(), Some("System prompt initialized."));
    assert_eq!(loaded.created_at(), session.created_at());
    assert_eq!(loaded.total_messages(), 4);

    // Verify messages content and structure
    assert_eq!(loaded.messages()[0].role, Role::User);
    assert_eq!(
        loaded.messages()[0].content,
        "Write a comprehensive smoke test for Fusion"
    );

    assert_eq!(loaded.messages()[1].role, Role::Assistant);
    assert!(loaded.messages()[1].tool_calls.is_some());
    let loaded_calls = loaded.messages()[1].tool_calls.as_ref().unwrap();
    assert_eq!(loaded_calls[0].id, "call_write_file_789");
    assert_eq!(loaded_calls[0].name, "write");

    assert_eq!(loaded.messages()[2].role, Role::Tool);
    assert_eq!(
        loaded.messages()[2].tool_call_id,
        Some("call_write_file_789".to_string())
    );

    assert_eq!(loaded.messages()[3].role, Role::Assistant);
    assert_eq!(
        loaded.messages()[3].content,
        "Smoke test implemented with 100% test coverage."
    );

    // Verify token stats
    assert_eq!(loaded.token_stats().prompt_tokens, 1200);
    assert_eq!(loaded.token_stats().completion_tokens, 480);
    assert_eq!(loaded.token_stats().total_tokens, 1680);

    // Verify metadata
    assert_eq!(loaded.get_metadata("env"), Some("staging"));
    assert_eq!(loaded.get_metadata("version"), Some("0.3.0"));
}

#[test]
fn test_session_modify_resave_and_reload() {
    let temp = tempdir().expect("Failed to create temporary directory");
    let file_path = temp.path().join("session_modify_cycle.json");

    // 1. Create and save initial session
    let mut session = Session::new("claude-3-5-sonnet");
    session.add_user_message("Turn 1 user request");
    session.add_assistant_message("Turn 1 assistant response");
    session
        .save_to_path(&file_path)
        .expect("Failed initial save");

    // 2. Reload and mutate
    let mut reloaded = Session::load_from_path(&file_path).expect("Failed initial reload");
    assert_eq!(reloaded.total_messages(), 2);

    reloaded.set_active_model("deepseek-reasoner");
    reloaded.add_user_message("Turn 2 user request");
    reloaded.add_assistant_message("Turn 2 assistant response");
    reloaded.set_working_dir(temp.path().to_path_buf());
    reloaded
        .save_to_path(&file_path)
        .expect("Failed second save");

    // 3. Reload again and verify mutations persisted
    let final_session =
        Session::load_from_path(&file_path).expect("Failed second reload");
    assert_eq!(final_session.active_model(), "deepseek-reasoner");
    assert_eq!(final_session.total_messages(), 4);
    assert_eq!(
        final_session.working_dir(),
        Some(temp.path())
    );
    assert_eq!(
        final_session.messages()[2].content,
        "Turn 2 user request"
    );
    assert_eq!(
        final_session.messages()[3].content,
        "Turn 2 assistant response"
    );
}

#[test]
fn test_session_delete_and_cleanup() {
    let temp = tempdir().expect("Failed to create temporary directory");
    let file_path = temp.path().join("session_to_delete.json");

    let session = Session::new("deepseek-chat");
    session
        .save_to_path(&file_path)
        .expect("Failed to save session");
    assert!(file_path.exists(), "Session file should exist");

    // Delete session
    Session::delete_by_path(&file_path).expect("Failed to delete session file");
    assert!(!file_path.exists(), "Session file should be removed from disk");

    // Loading deleted session should fail gracefully
    let load_res = Session::load_from_path(&file_path);
    assert!(load_res.is_err(), "Loading nonexistent session must return Err");
}

#[test]
fn test_session_summary_and_markdown_export() {
    let mut session = Session::new("deepseek-coder");
    session.add_user_message("How do I build a cross-platform TUI in Rust?");
    session.add_assistant_message("You can use Ratatui with Crossterm backend in inline mode.");
    session.record_usage(250, 95);

    // 1. SessionSummary representation
    let summary = SessionSummary {
        id: session.id,
        created_at: session.created_at.clone(),
        updated_at: session.updated_at.clone(),
        active_model: session.active_model.clone(),
        title: session.title.clone(),
        message_count: session.messages.len(),
        preview: "You can use Ratatui with Crossterm backend in inline mode.".to_string(),
    };

    assert_eq!(summary.message_count, 2);
    assert_eq!(summary.active_model, "deepseek-coder");
    assert!(summary.preview.contains("Ratatui"));

    // Verify summary JSON roundtrip
    let json_summary = serde_json::to_string(&summary).unwrap();
    let deser_summary: SessionSummary = serde_json::from_str(&json_summary).unwrap();
    assert_eq!(deser_summary.id, summary.id);
    assert_eq!(deser_summary.message_count, 2);

    // 2. Markdown Export
    let md_export = session.export_markdown();
    assert!(md_export.contains("# How do I build a cross-platform TUI in Rust?"));
    assert!(md_export.contains(&format!("- **Session ID:** `{}`", session.id)));
    assert!(md_export.contains("- **Model:** `deepseek-coder`"));
    assert!(md_export.contains("### 👤 User"));
    assert!(md_export.contains("How do I build a cross-platform TUI in Rust?"));
    assert!(md_export.contains("### 🤖 Assistant"));
    assert!(md_export.contains("You can use Ratatui with Crossterm"));
}

// ===========================================================================
// Part 5: Tool Execution Cycle Tests (Filesystem, Bash, Search)
// ===========================================================================

#[tokio::test]
async fn test_tool_registry_and_definitions() {
    let registry = default_registry();

    // Verify all primary tools and aliases exist in the registry
    assert!(registry.get("bash").is_some(), "bash tool missing");
    assert!(registry.get("read").is_some(), "read tool missing");
    assert!(registry.get("read_file").is_some(), "read_file alias missing");
    assert!(registry.get("write").is_some(), "write tool missing");
    assert!(registry.get("write_file").is_some(), "write_file alias missing");
    assert!(registry.get("edit").is_some(), "edit tool missing");
    assert!(registry.get("edit_file").is_some(), "edit_file alias missing");
    assert!(registry.get("grep").is_some(), "grep tool missing");
    assert!(registry.get("glob").is_some(), "glob tool missing");

    // Definitions list should contain the 6 registered tools
    let defs = registry.definitions();
    assert_eq!(defs.len(), 6, "Expected 6 tool definitions in default registry");

    let tool_names: Vec<String> = defs.into_iter().map(|d| d.name).collect();
    assert!(tool_names.contains(&"bash".to_string()));
    assert!(tool_names.contains(&"read".to_string()));
    assert!(tool_names.contains(&"write".to_string()));
    assert!(tool_names.contains(&"edit".to_string()));
    assert!(tool_names.contains(&"grep".to_string()));
    assert!(tool_names.contains(&"glob".to_string()));
}

#[tokio::test]
async fn test_tool_file_read_write_edit_cycle() {
    let temp = tempdir().expect("Failed to create tempdir for tool execution test");
    let ctx = ToolContext {
        cwd: temp.path().to_path_buf(),
        env: HashMap::new(),
    };

    let registry = default_registry();

    // 1. Write a new file in a nested directory via Registry
    let write_res = registry
        .execute(
            "write",
            json!({
                "path": "nested/dir/sample.txt",
                "content": "Line 1: Alpha\nLine 2: Beta\nLine 3: Gamma\nLine 4: Delta"
            }),
            &ctx,
        )
        .await;

    assert!(write_res.is_ok(), "Write tool execution failed: {:?}", write_res);
    let write_output = write_res.unwrap();
    assert!(write_output.contains("Successfully wrote"));

    // Verify file exists on disk
    let file_path = temp.path().join("nested/dir/sample.txt");
    assert!(file_path.exists(), "File was not created on disk");

    // 2. Read full file with line numbers
    let read_res = registry
        .execute(
            "read",
            json!({
                "path": "nested/dir/sample.txt",
                "line_numbers": true
            }),
            &ctx,
        )
        .await;

    assert!(read_res.is_ok(), "Read tool execution failed: {:?}", read_res);
    let read_output = read_res.unwrap();
    assert!(read_output.contains("1 | Line 1: Alpha"));
    assert!(read_output.contains("2 | Line 2: Beta"));
    assert!(read_output.contains("3 | Line 3: Gamma"));
    assert!(read_output.contains("4 | Line 4: Delta"));

    // 3. Read with offset and limit
    let read_slice_res = registry
        .execute(
            "read_file",
            json!({
                "path": "nested/dir/sample.txt",
                "offset": 2,
                "limit": 2,
                "line_numbers": false
            }),
            &ctx,
        )
        .await;

    assert!(read_slice_res.is_ok());
    let slice_output = read_slice_res.unwrap();
    assert!(!slice_output.contains("Line 1: Alpha"));
    assert!(slice_output.contains("Line 2: Beta"));
    assert!(slice_output.contains("Line 3: Gamma"));
    assert!(!slice_output.contains("Line 4: Delta"));

    // 4. Edit file: replace 'Line 2: Beta' with 'Line 2: Beta_Modified'
    let edit_res = registry
        .execute(
            "edit",
            json!({
                "path": "nested/dir/sample.txt",
                "old_str": "Line 2: Beta",
                "new_str": "Line 2: Beta_Modified"
            }),
            &ctx,
        )
        .await;

    assert!(edit_res.is_ok(), "Edit tool execution failed: {:?}", edit_res);
    let edit_output = edit_res.unwrap();
    assert!(edit_output.contains("Successfully edited"));

    // 5. Verify the edit by reading the modified file
    let verify_read = registry
        .execute(
            "read",
            json!({
                "path": "nested/dir/sample.txt",
                "line_numbers": false
            }),
            &ctx,
        )
        .await
        .unwrap();

    assert!(verify_read.contains("Line 2: Beta_Modified"));
    assert!(!verify_read.contains("Line 2: Beta\n"));
}

#[tokio::test]
async fn test_edit_tool_ambiguous_match_error() {
    let temp = tempdir().expect("Failed to create tempdir");
    let ctx = ToolContext {
        cwd: temp.path().to_path_buf(),
        env: HashMap::new(),
    };

    let registry = default_registry();

    // Create a file with duplicate lines
    let _ = registry
        .execute(
            "write",
            json!({
                "path": "duplicate.txt",
                "content": "target line\ntarget line\nother line"
            }),
            &ctx,
        )
        .await
        .unwrap();

    // Attempt to edit without unique match
    let edit_err = registry
        .execute(
            "edit",
            json!({
                "path": "duplicate.txt",
                "old_str": "target line",
                "new_str": "replacement line"
            }),
            &ctx,
        )
        .await;

    assert!(
        edit_err.is_err(),
        "Edit tool should fail when old_str matches multiple times"
    );
    let err_msg = edit_err.unwrap_err().to_string();
    assert!(
        err_msg.contains("2 times") || err_msg.contains("unique") || err_msg.contains("ambiguous"),
        "Error message should explain ambiguity: {}",
        err_msg
    );
}

#[tokio::test]
async fn test_bash_tool_execution() {
    let temp = tempdir().expect("Failed to create tempdir");
    let ctx = ToolContext {
        cwd: temp.path().to_path_buf(),
        env: HashMap::new(),
    };

    let registry = default_registry();

    // 1. Basic command output
    let echo_res = registry
        .execute(
            "bash",
            json!({
                "command": "echo 'Hello from Fusion Smoke Test'"
            }),
            &ctx,
        )
        .await;

    assert!(echo_res.is_ok(), "Bash tool echo failed: {:?}", echo_res);
    let echo_out = echo_res.unwrap();
    assert!(echo_out.contains("Hello from Fusion Smoke Test"));

    // 2. Working directory persistence and file creation via bash
    let create_res = registry
        .execute(
            "bash",
            json!({
                "command": "echo 'content from bash' > from_bash.txt && cat from_bash.txt"
            }),
            &ctx,
        )
        .await;

    assert!(create_res.is_ok());
    let create_out = create_res.unwrap();
    assert!(create_out.contains("content from bash"));
    assert!(temp.path().join("from_bash.txt").exists());

    // 3. Command failure status code
    let fail_res = registry
        .execute(
            "bash",
            json!({
                "command": "ls /non_existent_path_xyz_123456789"
            }),
            &ctx,
        )
        .await;

    // Bash tool captures stderr/exit code in output or returns error
    match fail_res {
        Ok(output) => {
            assert!(
                output.contains("No such file") || output.contains("exit status") || output.contains("failed") || output.contains("cannot access"),
                "Output should report command failure: {}",
                output
            );
        }
        Err(e) => {
            let err_str = e.to_string();
            assert!(
                err_str.contains("failed") || err_str.contains("No such file") || err_str.contains("exit"),
                "Error should report failure: {}",
                err_str
            );
        }
    }
}

#[tokio::test]
async fn test_search_tools_grep_and_glob() {
    let temp = tempdir().expect("Failed to create tempdir");
    let ctx = ToolContext {
        cwd: temp.path().to_path_buf(),
        env: HashMap::new(),
    };

    let registry = default_registry();

    // Populate directory structure for searching
    let _ = registry
        .execute(
            "write",
            json!({
                "path": "src/main.rs",
                "content": "fn main() {\n    println!(\"Hello Fusion!\");\n}"
            }),
            &ctx,
        )
        .await
        .unwrap();

    let _ = registry
        .execute(
            "write",
            json!({
                "path": "src/lib.rs",
                "content": "pub fn hello_world() -> &'static str {\n    \"Hello Fusion!\"\n}"
            }),
            &ctx,
        )
        .await
        .unwrap();

    let _ = registry
        .execute(
            "write",
            json!({
                "path": "tests/test_mod.rs",
                "content": "#[test]\nfn test_something() {\n    assert!(true);\n}"
            }),
            &ctx,
        )
        .await
        .unwrap();

    // 1. Glob search for *.rs files
    let glob_res = registry
        .execute(
            "glob",
            json!({
                "pattern": "**/*.rs"
            }),
            &ctx,
        )
        .await;

    assert!(glob_res.is_ok(), "Glob execution failed: {:?}", glob_res);
    let glob_out = glob_res.unwrap();
    assert!(glob_out.contains("src/main.rs") || glob_out.contains("main.rs"));
    assert!(glob_out.contains("src/lib.rs") || glob_out.contains("lib.rs"));
    assert!(glob_out.contains("tests/test_mod.rs") || glob_out.contains("test_mod.rs"));

    // 2. Grep search for "Hello Fusion!"
    let grep_res = registry
        .execute(
            "grep",
            json!({
                "pattern": "Hello Fusion!"
            }),
            &ctx,
        )
        .await;

    assert!(grep_res.is_ok(), "Grep execution failed: {:?}", grep_res);
    let grep_out = grep_res.unwrap();
    assert!(grep_out.contains("src/main.rs") || grep_out.contains("main.rs"));
    assert!(grep_out.contains("src/lib.rs") || grep_out.contains("lib.rs"));
    assert!(!grep_out.contains("test_mod.rs"));
}
