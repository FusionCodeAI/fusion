//! Comprehensive tests for local daemon detection and probing engine:
//! - Antigravity Tools GUI config parsing (`gui_config.json`)
//! - Codex CLI authentication parsing (`auth.json`)
//! - Model list parsing and mapping to `CatalogModel`
//! - Port accessibility probing and fallback handling
//! - Live endpoint probing and network error resilience

use std::fs;
use std::net::TcpListener;
use std::time::Duration;

use tempfile::tempdir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use fusion::provider::local_daemon::{
    check_port_accessible, default_antigravity_config_path, default_codex_auth_path,
    detect_antigravity_daemon, detect_antigravity_daemon_at, detect_codex_auth,
    detect_codex_auth_at, fetch_antigravity_models, parse_antigravity_config,
    parse_antigravity_models_json, parse_codex_auth, probe_local_daemons,
    probe_local_daemons_from_paths, LocalDaemonEndpoint, DEFAULT_ANTIGRAVITY_BASE_URL,
    DEFAULT_ANTIGRAVITY_PORT, DEFAULT_CODEX_BASE_URL,
};

// ---------------------------------------------------------------------------
// 1. Antigravity Tools JSON Parsing Tests
// ---------------------------------------------------------------------------

#[test]
fn test_parse_antigravity_gui_config_full() {
    let json = r#"{
        "language": "en",
        "theme": "dark",
        "proxy": {
            "enabled": true,
            "allow_lan_access": false,
            "auth_mode": "auto",
            "port": 9000,
            "api_key": "sk-test-secret-key-123"
        },
        "auto_launch": true
    }"#;

    let config = parse_antigravity_config(json).expect("Failed to parse valid config");
    assert_eq!(config.auto_launch, Some(true));

    let proxy = config.proxy.expect("proxy field missing");
    assert_eq!(proxy.enabled, Some(true));
    assert_eq!(proxy.port, Some(9000));
    assert_eq!(proxy.api_key.as_deref(), Some("sk-test-secret-key-123"));
}

#[test]
fn test_parse_antigravity_gui_config_defaults() {
    let json = r#"{
        "proxy": {}
    }"#;

    let config = parse_antigravity_config(json).expect("Failed to parse minimal config");
    let proxy = config.proxy.expect("proxy field missing");
    assert_eq!(proxy.port, None);
    assert_eq!(proxy.api_key, None);
}

#[test]
fn test_parse_antigravity_gui_config_empty() {
    let json = "{}";
    let config = parse_antigravity_config(json).expect("Failed to parse empty object");
    assert!(config.proxy.is_none());
}

#[test]
fn test_parse_antigravity_gui_config_malformed() {
    let json = "{ not valid json";
    assert!(parse_antigravity_config(json).is_err());
}

#[test]
fn test_parse_antigravity_gui_config_real_world() {
    let json = r#"{
        "language": "en",
        "theme": "light",
        "auto_refresh": false,
        "proxy": {
            "enabled": false,
            "allow_lan_access": true,
            "auth_mode": "auto",
            "port": 8045,
            "api_key": "sk-d0c684859e1c44a59feb5986b91a8d15",
            "admin_password": null,
            "request_timeout": 120
        },
        "antigravity_executable": "/Applications/Antigravity IDE.app"
    }"#;

    let config = parse_antigravity_config(json).expect("Real world config must parse");
    let proxy = config.proxy.unwrap();
    assert_eq!(proxy.port, Some(8045));
    assert_eq!(
        proxy.api_key.as_deref(),
        Some("sk-d0c684859e1c44a59feb5986b91a8d15")
    );
}

// ---------------------------------------------------------------------------
// 2. Codex CLI Authentication Parsing Tests
// ---------------------------------------------------------------------------

#[test]
fn test_parse_codex_auth_full() {
    let json = r#"{
        "auth_mode": "chatgpt",
        "OPENAI_API_KEY": null,
        "tokens": {
            "id_token": "id-jwt-xyz",
            "access_token": "access-jwt-abc-123",
            "refresh_token": "refresh-token-rt-789",
            "account_id": "acc-uuid-456"
        },
        "last_refresh": "2026-08-14T09:54:11.493646Z"
    }"#;

    let auth = parse_codex_auth(json).expect("Failed to parse valid auth.json");
    assert_eq!(auth.auth_mode.as_deref(), Some("chatgpt"));

    let tokens = auth.tokens.expect("tokens missing");
    assert_eq!(tokens.access_token.as_deref(), Some("access-jwt-abc-123"));
    assert_eq!(tokens.account_id.as_deref(), Some("acc-uuid-456"));
    assert_eq!(tokens.refresh_token.as_deref(), Some("refresh-token-rt-789"));
}

#[test]
fn test_parse_codex_auth_missing_tokens() {
    let json = r#"{ "auth_mode": "chatgpt" }"#;
    let auth = parse_codex_auth(json).expect("Parsed without tokens");
    assert!(auth.tokens.is_none());
}

#[test]
fn test_parse_codex_auth_malformed() {
    let json = "{ broken json content";
    assert!(parse_codex_auth(json).is_err());
}

// ---------------------------------------------------------------------------
// 3. Antigravity Detection & Port Checking Tests
// ---------------------------------------------------------------------------

#[test]
fn test_detect_antigravity_missing_file() {
    let dir = tempdir().unwrap();
    let missing_path = dir.path().join("does_not_exist.json");
    let result = detect_antigravity_daemon_at(&missing_path);
    assert!(result.is_none());
}

#[test]
fn test_detect_antigravity_invalid_json() {
    let dir = tempdir().unwrap();
    let bad_path = dir.path().join("bad.json");
    fs::write(&bad_path, "invalid json").unwrap();
    let result = detect_antigravity_daemon_at(&bad_path);
    assert!(result.is_none());
}

#[test]
fn test_detect_antigravity_default_port_fallback() {
    let dir = tempdir().unwrap();
    let config_path = dir.path().join("gui_config.json");
    // Proxy with no port specified -> should default to 8045
    fs::write(
        &config_path,
        r#"{"proxy": {"api_key": "sk-fallback-test"}}"#,
    )
    .unwrap();

    let endpoint = detect_antigravity_daemon_at(&config_path)
        .expect("Should produce endpoint with default port");
    assert_eq!(endpoint.provider, "antigravity");
    assert_eq!(
        endpoint.base_url,
        format!("http://127.0.0.1:{}/v1", DEFAULT_ANTIGRAVITY_PORT)
    );
    assert_eq!(endpoint.api_key, "sk-fallback-test");
    assert_eq!(endpoint.account_id, None);
}

#[test]
fn test_detect_antigravity_with_live_listener() {
    // Bind to an ephemeral port to guarantee port accessibility is tested
    let listener = TcpListener::bind("127.0.0.1:0").expect("Failed to bind ephemeral port");
    let port = listener.local_addr().unwrap().port();

    let dir = tempdir().unwrap();
    let config_path = dir.path().join("gui_config.json");
    fs::write(
        &config_path,
        format!(
            r#"{{
                "proxy": {{
                    "port": {},
                    "api_key": "sk-live-test-key"
                }}
            }}"#,
            port
        ),
    )
    .unwrap();

    let endpoint = detect_antigravity_daemon_at(&config_path)
        .expect("Should detect live endpoint");

    assert_eq!(endpoint.provider, "antigravity");
    assert_eq!(endpoint.base_url, format!("http://127.0.0.1:{}/v1", port));
    assert_eq!(endpoint.api_key, "sk-live-test-key");
    assert!(endpoint.is_alive, "Expected is_alive to be true for open port");

    drop(listener);

    // After dropping listener, port accessibility check returns false
    let port_open = check_port_accessible(port, Duration::from_millis(100));
    assert!(!port_open, "Closed port should not be accessible");
}

#[test]
fn test_check_port_accessible_closed_port() {
    // Port 1 is reserved / almost certainly not listening on localhost
    let accessible = check_port_accessible(1, Duration::from_millis(50));
    assert!(!accessible);
}

// ---------------------------------------------------------------------------
// 4. Codex Detection Tests
// ---------------------------------------------------------------------------

#[test]
fn test_detect_codex_missing_file() {
    let dir = tempdir().unwrap();
    let missing_path = dir.path().join("auth.json");
    let result = detect_codex_auth_at(&missing_path);
    assert!(result.is_none());
}

#[test]
fn test_detect_codex_missing_access_token() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("auth.json");
    fs::write(
        &path,
        r#"{
            "tokens": {
                "account_id": "acc-123"
            }
        }"#,
    )
    .unwrap();

    let result = detect_codex_auth_at(&path);
    assert!(result.is_none(), "Must return None when access_token is missing");
}

#[test]
fn test_detect_codex_empty_access_token() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("auth.json");
    fs::write(
        &path,
        r#"{
            "tokens": {
                "access_token": "   ",
                "account_id": "acc-123"
            }
        }"#,
    )
    .unwrap();

    let result = detect_codex_auth_at(&path);
    assert!(result.is_none(), "Must return None when access_token is whitespace");
}

#[test]
fn test_detect_codex_valid() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("auth.json");
    fs::write(
        &path,
        r#"{
            "tokens": {
                "access_token": "eyJhbGciOi...",
                "account_id": "account-uuid-999"
            }
        }"#,
    )
    .unwrap();

    let endpoint = detect_codex_auth_at(&path).expect("Should detect valid codex auth");
    assert_eq!(endpoint.provider, "codex");
    assert_eq!(endpoint.base_url, DEFAULT_CODEX_BASE_URL);
    assert_eq!(endpoint.api_key, "eyJhbGciOi...");
    assert_eq!(endpoint.account_id.as_deref(), Some("account-uuid-999"));
    assert!(endpoint.is_alive);
}

// ---------------------------------------------------------------------------
// 5. Model Parsing & Catalog Mapping Tests
// ---------------------------------------------------------------------------

#[test]
fn test_parse_antigravity_models_json_mapping() {
    let json = r#"{
        "object": "list",
        "data": [
            {
                "id": "claude-opus-4-6-thinking",
                "object": "model",
                "name": "Claude Opus 4.6 Thinking"
            },
            {
                "id": "claude-sonnet-4-6",
                "object": "model"
            },
            {
                "id": "gemini-3.8-flash-high",
                "object": "model"
            }
        ]
    }"#;

    let models = parse_antigravity_models_json(json).expect("Failed to parse models json");
    assert_eq!(models.len(), 3);

    // Check Opus model
    let opus = &models[0];
    assert_eq!(opus.id, "claude-opus-4-6-thinking");
    assert_eq!(opus.name, "Claude Opus 4.6 Thinking");
    assert_eq!(opus.provider, "antigravity");
    assert!(opus.badges.contains(&"Local Daemon (Free)".to_string()));
    assert_eq!(opus.input_cost_per_m, Some(0.0));
    assert_eq!(opus.output_cost_per_m, Some(0.0));
    assert!(opus.capabilities.reasoning, "Thinking model should have reasoning flag");
    assert!(opus.capabilities.vision, "Claude model should have vision flag");

    // Check Sonnet model (name defaults to id when missing)
    let sonnet = &models[1];
    assert_eq!(sonnet.id, "claude-sonnet-4-6");
    assert_eq!(sonnet.name, "claude-sonnet-4-6");
    assert_eq!(sonnet.provider, "antigravity");
    assert!(sonnet.badges.contains(&"Local Daemon (Free)".to_string()));

    // Check Gemini model
    let gemini = &models[2];
    assert_eq!(gemini.id, "gemini-3.8-flash-high");
    assert_eq!(gemini.provider, "antigravity");
    assert!(gemini.badges.contains(&"Local Daemon (Free)".to_string()));
    assert!(gemini.capabilities.vision);
}

#[test]
fn test_parse_antigravity_models_empty_data() {
    let json = r#"{"object": "list", "data": []}"#;
    let models = parse_antigravity_models_json(json).expect("Empty list should parse");
    assert!(models.is_empty());
}

#[test]
fn test_parse_antigravity_models_invalid_json() {
    let json = "not json";
    assert!(parse_antigravity_models_json(json).is_err());
}

// ---------------------------------------------------------------------------
// 6. Probing & Fallback Precedence Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_probe_local_daemons_from_paths_both_present() {
    let dir = tempdir().unwrap();

    // Live listener for antigravity
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let ag_port = listener.local_addr().unwrap().port();

    let ag_path = dir.path().join("gui_config.json");
    fs::write(
        &ag_path,
        format!(r#"{{"proxy": {{"port": {}, "api_key": "sk-ag"}}}}"#, ag_port),
    )
    .unwrap();

    let codex_path = dir.path().join("auth.json");
    fs::write(
        &codex_path,
        r#"{"tokens": {"access_token": "token-cdx", "account_id": "acc-1"}}"#,
    )
    .unwrap();

    let endpoints = probe_local_daemons_from_paths(Some(&ag_path), Some(&codex_path)).await;
    assert_eq!(endpoints.len(), 2);
    assert_eq!(endpoints[0].provider, "antigravity");
    assert_eq!(endpoints[1].provider, "codex");
    assert_eq!(endpoints[1].account_id.as_deref(), Some("acc-1"));
}

#[tokio::test]
async fn test_probe_local_daemons_antigravity_offline_fallback_to_codex() {
    let dir = tempdir().unwrap();

    // Closed port for Antigravity (daemon not running)
    let ag_path = dir.path().join("gui_config.json");
    fs::write(
        &ag_path,
        r#"{"proxy": {"port": 1, "api_key": "sk-ag"}}"#,
    )
    .unwrap();

    let codex_path = dir.path().join("auth.json");
    fs::write(
        &codex_path,
        r#"{"tokens": {"access_token": "token-cdx"}}"#,
    )
    .unwrap();

    let endpoints = probe_local_daemons_from_paths(Some(&ag_path), Some(&codex_path)).await;
    // Antigravity is offline (port 1 is closed), so only Codex is available
    assert_eq!(endpoints.len(), 1);
    assert_eq!(endpoints[0].provider, "codex");
    assert_eq!(endpoints[0].api_key, "token-cdx");
}

#[tokio::test]
async fn test_probe_local_daemons_neither_present() {
    let endpoints = probe_local_daemons_from_paths(None, None).await;
    assert!(endpoints.is_empty());
}

// ---------------------------------------------------------------------------
// 7. Live Mock HTTP Server Model Fetching Test
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_fetch_antigravity_models_mock_http() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    tokio::spawn(async move {
        if let Ok((mut socket, _)) = listener.accept().await {
            let mut buf = [0u8; 2048];
            let n = socket.read(&mut buf).await.unwrap_or(0);
            let req_str = String::from_utf8_lossy(&buf[..n]);

            // Verify requested path and authorization header
            assert!(req_str.starts_with("GET /models") || req_str.starts_with("GET /v1/models") || req_str.contains("/models"));
            assert!(req_str.contains("authorization: Bearer sk-mock-key") || req_str.contains("Authorization: Bearer sk-mock-key"));

            let response_body = r#"{
                "object": "list",
                "data": [
                    {"id": "claude-opus-4-6"},
                    {"id": "gemini-3.8-flash-high"}
                ]
            }"#;

            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            let _ = socket.write_all(response.as_bytes()).await;
        }
    });

    let endpoint = LocalDaemonEndpoint::new(
        "antigravity",
        format!("http://127.0.0.1:{}", port),
        "sk-mock-key",
        None,
        true,
    );

    let models = fetch_antigravity_models(&endpoint).await;
    assert_eq!(models.len(), 2);
    assert_eq!(models[0].id, "claude-opus-4-6");
    assert_eq!(models[0].provider, "antigravity");
    assert!(models[0].badges.contains(&"Local Daemon (Free)".to_string()));
    assert_eq!(models[1].id, "gemini-3.8-flash-high");
}

#[tokio::test]
async fn test_fetch_antigravity_models_network_error_fallback() {
    // Port 1 closed -> network connection refused
    let endpoint = LocalDaemonEndpoint::new(
        "antigravity",
        "http://127.0.0.1:1/v1",
        "sk-test",
        None,
        false,
    );

    let models = fetch_antigravity_models(&endpoint).await;
    assert!(models.is_empty(), "Network failure should return empty Vec");
}

// ---------------------------------------------------------------------------
// 8. Integration with Environment / System Daemons
// ---------------------------------------------------------------------------

#[test]
fn test_default_paths_detectable() {
    // Verifies helper functions return valid Option without panicking
    let _ag_path = default_antigravity_config_path();
    let _cdx_path = default_codex_auth_path();
}

#[test]
fn test_system_antigravity_or_codex_detection() {
    // If running on developer's workstation with ~/.antigravity_tools or ~/.codex
    if let Some(ep) = detect_antigravity_daemon() {
        assert_eq!(ep.provider, "antigravity");
        assert!(ep.base_url.starts_with("http://127.0.0.1:"));
    }

    if let Some(ep) = detect_codex_auth() {
        assert_eq!(ep.provider, "codex");
        assert_eq!(ep.base_url, DEFAULT_CODEX_BASE_URL);
        assert!(!ep.api_key.is_empty());
    }
}

#[tokio::test]
async fn test_system_probe_local_daemons_and_constants() {
    assert_eq!(DEFAULT_ANTIGRAVITY_BASE_URL, "http://127.0.0.1:8045/v1");
    let endpoints = probe_local_daemons().await;
    for ep in &endpoints {
        assert!(ep.is_alive);
        assert!(!ep.base_url.is_empty());
        assert!(!ep.provider.is_empty());
    }
}
