//! Comprehensive integration and unit tests for the Browser Automation Tool.
//!
//! Tests:
//! 1. Action enum serialization, deserialization, aliases, and string conversion.
//! 2. Argument JSON parsing and implicit action inference.
//! 3. Tool trait metadata, definition, and parameter JSON schema compliance.
//! 4. RFC 4648 Base64 encoding, decoding, and roundtrips.
//! 5. DOM reader text extraction, entity decoding, and HTML cleaning.
//! 6. URL normalization and WebSocket URL parsing.
//! 7. Fallback behavior when CDP is offline (advisory notice & CLI flags).
//! 8. Parameter validation errors (missing selector, text, code, url).
//! 9. Mock CDP HTTP management server (`/json/version`, `/json/list`).
//! 10. Mock CDP WebSocket handshake and JSON-RPC dispatch over TCP.

#[allow(unused_imports)]
pub mod tools {
    pub use fusion::tools::*;
}

#[path = "../src/tools/browser.rs"]
pub mod browser;

use browser::*;
use fusion::tools::Tool;
use fusion::tools::ToolContext;
use serde_json::json;
use std::str::FromStr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

// ============================================================================
// 1. BrowserAction Enum Tests
// ============================================================================

#[test]
fn test_browser_action_serialization_and_deserialization() {
    let actions = [
        (BrowserAction::Open, "open"),
        (BrowserAction::Navigate, "navigate"),
        (BrowserAction::Screenshot, "screenshot"),
        (BrowserAction::Click, "click"),
        (BrowserAction::Type, "type"),
        (BrowserAction::EvaluateJs, "evaluate_js"),
        (BrowserAction::Content, "content"),
        (BrowserAction::Close, "close"),
    ];

    for (action, expected_str) in actions {
        // Serialization
        let serialized = serde_json::to_string(&action).expect("action should serialize");
        assert_eq!(serialized, format!("\"{}\"", expected_str));

        // Deserialization
        let deserialized: BrowserAction =
            serde_json::from_str(&serialized).expect("action should deserialize");
        assert_eq!(deserialized, action);

        // as_str & Display
        assert_eq!(action.as_str(), expected_str);
        assert_eq!(action.to_string(), expected_str);

        // FromStr
        let parsed = BrowserAction::from_str(expected_str).expect("from_str should succeed");
        assert_eq!(parsed, action);
    }
}

#[test]
fn test_browser_action_aliases_and_case_insensitivity() {
    // evaluate_js aliases
    assert_eq!(
        BrowserAction::from_str("evaluate").unwrap(),
        BrowserAction::EvaluateJs
    );
    assert_eq!(
        BrowserAction::from_str("eval").unwrap(),
        BrowserAction::EvaluateJs
    );
    assert_eq!(
        BrowserAction::from_str("eval_js").unwrap(),
        BrowserAction::EvaluateJs
    );
    assert_eq!(
        BrowserAction::from_str("js").unwrap(),
        BrowserAction::EvaluateJs
    );

    // navigate aliases
    assert_eq!(
        BrowserAction::from_str("nav").unwrap(),
        BrowserAction::Navigate
    );
    assert_eq!(
        BrowserAction::from_str("goto").unwrap(),
        BrowserAction::Navigate
    );

    // screenshot aliases
    assert_eq!(
        BrowserAction::from_str("capture").unwrap(),
        BrowserAction::Screenshot
    );
    assert_eq!(
        BrowserAction::from_str("snap").unwrap(),
        BrowserAction::Screenshot
    );
    assert_eq!(
        BrowserAction::from_str("screen").unwrap(),
        BrowserAction::Screenshot
    );

    // click & type aliases
    assert_eq!(
        BrowserAction::from_str("input").unwrap(),
        BrowserAction::Type
    );
    assert_eq!(
        BrowserAction::from_str("fill").unwrap(),
        BrowserAction::Type
    );

    // content aliases
    assert_eq!(
        BrowserAction::from_str("read").unwrap(),
        BrowserAction::Content
    );
    assert_eq!(
        BrowserAction::from_str("dom").unwrap(),
        BrowserAction::Content
    );
    assert_eq!(
        BrowserAction::from_str("text").unwrap(),
        BrowserAction::Content
    );

    // close aliases
    assert_eq!(
        BrowserAction::from_str("quit").unwrap(),
        BrowserAction::Close
    );
    assert_eq!(
        BrowserAction::from_str("exit").unwrap(),
        BrowserAction::Close
    );

    // Case insensitivity & trimming
    assert_eq!(
        BrowserAction::from_str("  OPEN  ").unwrap(),
        BrowserAction::Open
    );
    assert_eq!(
        BrowserAction::from_str("Navigate").unwrap(),
        BrowserAction::Navigate
    );

    // Unknown action returns error
    assert!(BrowserAction::from_str("unknown_action_xyz").is_err());
}

// ============================================================================
// 2. Argument JSON Parsing & Inference Tests
// ============================================================================

#[test]
fn test_parse_browser_args_explicit() {
    let raw = json!({
        "action": "screenshot",
        "url": "https://example.com",
        "selector": "#main-header",
        "screenshot_path": "header.png",
        "cdp_url": "http://127.0.0.1:9222"
    });

    let args = parse_browser_args(&raw).expect("should parse browser args");
    assert_eq!(args.action, Some(BrowserAction::Screenshot));
    assert_eq!(args.url.as_deref(), Some("https://example.com"));
    assert_eq!(args.selector.as_deref(), Some("#main-header"));
    assert_eq!(args.screenshot_path.as_deref(), Some("header.png"));
    assert_eq!(args.cdp_url.as_deref(), Some("http://127.0.0.1:9222"));
}

#[test]
fn test_parse_browser_args_inference() {
    // URL without action -> Open
    let raw_url = json!({ "url": "https://rust-lang.org" });
    let args_url = parse_browser_args(&raw_url).unwrap();
    assert_eq!(args_url.action, Some(BrowserAction::Open));
    assert_eq!(args_url.url.as_deref(), Some("https://rust-lang.org"));

    // Code without action -> EvaluateJs
    let raw_code = json!({ "code": "document.title" });
    let args_code = parse_browser_args(&raw_code).unwrap();
    assert_eq!(args_code.action, Some(BrowserAction::EvaluateJs));
    assert_eq!(args_code.code.as_deref(), Some("document.title"));

    // Selector + text without action -> Type
    let raw_type = json!({ "selector": "input#search", "text": "tokio" });
    let args_type = parse_browser_args(&raw_type).unwrap();
    assert_eq!(args_type.action, Some(BrowserAction::Type));
    assert_eq!(args_type.text.as_deref(), Some("tokio"));

    // Selector only without action -> Click
    let raw_click = json!({ "selector": "button#submit" });
    let args_click = parse_browser_args(&raw_click).unwrap();
    assert_eq!(args_click.action, Some(BrowserAction::Click));
    assert_eq!(args_click.selector.as_deref(), Some("button#submit"));
}

// ============================================================================
// 3. Tool Trait Metadata and Schema Tests
// ============================================================================

#[test]
fn test_browser_tool_name_and_description() {
    let tool = BrowserTool::new();
    assert_eq!(tool.name(), "browser");

    let desc = tool.description();
    assert!(!desc.is_empty());
    assert!(
        desc.contains("Chrome DevTools Protocol") || desc.contains("CDP"),
        "Description should mention Chrome DevTools Protocol or CDP"
    );
    assert!(
        desc.contains("screenshot") || desc.contains("DOM"),
        "Description should mention screenshotting or DOM"
    );

    let def = tool.definition();
    assert_eq!(def.name, "browser");
    assert_eq!(def.description, tool.description());
    assert_eq!(def.parameters, tool.parameters());
}

#[test]
fn test_browser_tool_parameters_schema() {
    let tool = BrowserTool::new();
    let schema = tool.parameters();

    assert_eq!(
        schema.get("type").and_then(|v| v.as_str()),
        Some("object"),
        "Top-level schema must be object"
    );

    let required = schema
        .get("required")
        .and_then(|v| v.as_array())
        .expect("parameters should specify required list");
    assert!(
        required.iter().any(|v| v.as_str() == Some("action")),
        "'action' must be in required properties"
    );

    let props = schema
        .get("properties")
        .and_then(|v| v.as_object())
        .expect("properties object must exist");

    // Check all required property keys
    assert!(props.contains_key("action"));
    assert!(props.contains_key("url"));
    assert!(props.contains_key("selector"));
    assert!(props.contains_key("text"));
    assert!(props.contains_key("code"));
    assert!(props.contains_key("cdp_url"));
    assert!(props.contains_key("screenshot_path"));

    // Check action enum list contains all 8 actions
    let action_prop = props.get("action").and_then(|v| v.as_object()).unwrap();
    let action_enum = action_prop
        .get("enum")
        .and_then(|v| v.as_array())
        .expect("action property must specify enum list");

    let enum_strings: Vec<&str> = action_enum.iter().filter_map(|v| v.as_str()).collect();
    for expected in [
        "open",
        "navigate",
        "screenshot",
        "click",
        "type",
        "evaluate_js",
        "content",
        "close",
    ] {
        assert!(
            enum_strings.contains(&expected),
            "Enum list must contain '{}'",
            expected
        );
    }
}

// ============================================================================
// 4. Base64 RFC 4648 Encoding & Decoding Tests
// ============================================================================

#[test]
fn test_base64_rfc4648_test_vectors() {
    let vectors = [
        ("", ""),
        ("f", "Zg=="),
        ("fo", "Zm8="),
        ("foo", "Zm9v"),
        ("foob", "Zm9vYg=="),
        ("fooba", "Zm9vYmE="),
        ("foobar", "Zm9vYmFy"),
        ("Hello, World!", "SGVsbG8sIFdvcmxkIQ=="),
    ];

    for (plain, encoded) in vectors {
        assert_eq!(base64_encode(plain.as_bytes()), encoded);
        let decoded = base64_decode(encoded).expect("should decode valid base64");
        assert_eq!(String::from_utf8_lossy(&decoded), plain);
    }
}

#[test]
fn test_base64_arbitrary_binary_roundtrip() {
    let mut bytes = Vec::new();
    for i in 0..=255u8 {
        bytes.push(i);
    }

    let encoded = base64_encode(&bytes);
    let decoded = base64_decode(&encoded).expect("should decode binary base64");
    assert_eq!(decoded, bytes);
}

#[test]
fn test_base64_decode_invalid_inputs() {
    // Non-multiple of 4
    assert!(base64_decode("ABC").is_none());
    assert!(base64_decode("A").is_none());

    // Invalid character
    assert!(base64_decode("ABC!").is_none());

    // Misplaced padding
    assert!(base64_decode("=ABC").is_none());
}

// ============================================================================
// 5. DOM Reader & Entity Unescaping Tests
// ============================================================================

#[test]
fn test_extract_title_from_html() {
    let html = r#"<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <title>Fusion Autonomous Agent &mdash; Fast &amp; Light</title>
</head>
<body>
    <h1>Welcome</h1>
</body>
</html>"#;

    let title = extract_title(html).expect("title should be found");
    assert_eq!(title, "Fusion Autonomous Agent — Fast & Light");
}

#[test]
fn test_html_reader_text_extraction() {
    let html = r#"
    <div>
        <h1>Heading 1</h1>
        <p>Paragraph with <a href="https://example.com">link</a> and <b>bold</b> text.</p>
        <ul>
            <li>Item Alpha</li>
            <li>Item Beta</li>
        </ul>
        <script>console.log("ignore script");</script>
        <style>body { color: red; }</style>
        <!-- HTML comment -->
        <p>Footer note &copy; 2026</p>
    </div>
    "#;

    let text = html_to_readable_text(html);

    assert!(text.contains("Heading 1"));
    assert!(text.contains("Paragraph with"));
    assert!(text.contains("link"));
    assert!(text.contains("Item Alpha"));
    assert!(text.contains("Item Beta"));
    assert!(text.contains("Footer note © 2026"));

    // Scripts and styles must be stripped
    assert!(!text.contains("console.log"));
    assert!(!text.contains("color: red"));
    assert!(!text.contains("HTML comment"));
}

#[test]
fn test_unescape_html_entities() {
    let raw = "&quot;Hello &amp; Welcome&quot; &lt;Fusion&gt; &#39;v2&#39; &copy; &mdash;";
    let unescaped = unescape_html_entities(raw);
    assert_eq!(unescaped, "\"Hello & Welcome\" <Fusion> 'v2' © —");
}

// ============================================================================
// 6. URL Normalization and WebSocket Parsing Tests
// ============================================================================

#[test]
fn test_url_normalization() {
    assert_eq!(normalize_url("example.com"), "https://example.com");
    assert_eq!(normalize_url("http://localhost:8080"), "http://localhost:8080");
    assert_eq!(normalize_url("https://fusion.ai"), "https://fusion.ai");
    assert_eq!(normalize_url("about:blank"), "about:blank");
}

#[test]
fn test_parse_ws_url() {
    let (host, port, path) =
        parse_ws_url("ws://127.0.0.1:9222/devtools/page/ABC123").unwrap();
    assert_eq!(host, "127.0.0.1");
    assert_eq!(port, 9222);
    assert_eq!(path, "/devtools/page/ABC123");

    let (host2, port2, path2) =
        parse_ws_url("ws://localhost:9222/devtools/browser/xyz").unwrap();
    assert_eq!(host2, "localhost");
    assert_eq!(port2, 9222);
    assert_eq!(path2, "/devtools/browser/xyz");

    let (host3, port3, path3) = parse_ws_url("http://127.0.0.1:9222").unwrap();
    assert_eq!(host3, "127.0.0.1");
    assert_eq!(port3, 9222);
    assert_eq!(path3, "/");
}

// ============================================================================
// 7. Fallback Error Advice on Offline CDP Tests
// ============================================================================

#[tokio::test]
async fn test_fallback_advisories_when_cdp_offline() {
    let tool = BrowserTool::new().with_cdp_url("http://127.0.0.1:59999");
    let ctx = ToolContext::default();

    // Screenshot when CDP is offline gives startup advice
    let res_screen = tool
        .execute(
            json!({
                "action": "screenshot",
                "cdp_url": "http://127.0.0.1:59999"
            }),
            &ctx,
        )
        .await
        .expect("should return advisory report");
    assert!(res_screen.contains("Screenshot Unavailable"));
    assert!(res_screen.contains("--remote-debugging-port=9222"));

    // Click when CDP is offline gives startup advice
    let res_click = tool
        .execute(
            json!({
                "action": "click",
                "selector": "#login-btn",
                "cdp_url": "http://127.0.0.1:59999"
            }),
            &ctx,
        )
        .await
        .expect("should return advisory report");
    assert!(res_click.contains("Click Action Unavailable"));
    assert!(res_click.contains("--remote-debugging-port=9222"));

    // Type when CDP is offline gives startup advice
    let res_type = tool
        .execute(
            json!({
                "action": "type",
                "selector": "input#query",
                "text": "Fusion test",
                "cdp_url": "http://127.0.0.1:59999"
            }),
            &ctx,
        )
        .await
        .expect("should return advisory report");
    assert!(res_type.contains("Type Action Unavailable"));
    assert!(res_type.contains("--remote-debugging-port=9222"));

    // EvaluateJs when CDP is offline gives startup advice
    let res_eval = tool
        .execute(
            json!({
                "action": "evaluate_js",
                "code": "1 + 1",
                "cdp_url": "http://127.0.0.1:59999"
            }),
            &ctx,
        )
        .await
        .expect("should return advisory report");
    assert!(res_eval.contains("JavaScript Evaluation Unavailable"));
    assert!(res_eval.contains("--remote-debugging-port=9222"));

    // Close when CDP is offline returns clean report
    let res_close = tool
        .execute(
            json!({
                "action": "close",
                "cdp_url": "http://127.0.0.1:59999"
            }),
            &ctx,
        )
        .await
        .expect("should return clean report");
    assert!(res_close.contains("Browser Session Closed"));
}

// ============================================================================
// 8. Parameter Validation Error Tests
// ============================================================================

#[tokio::test]
async fn test_parameter_validation_errors() {
    let tool = BrowserTool::new();
    let ctx = ToolContext::default();

    // Missing action
    let err_no_action = tool.execute(json!({}), &ctx).await;
    assert!(err_no_action.is_err());
    assert!(err_no_action
        .unwrap_err()
        .to_string()
        .contains("Missing required parameter 'action'"));

    // Click missing selector
    let err_click = tool.execute(json!({ "action": "click" }), &ctx).await;
    assert!(err_click.is_err());
    assert!(err_click.unwrap_err().to_string().contains("selector"));

    // Type missing text
    let err_type = tool
        .execute(
            json!({ "action": "type", "selector": "#username" }),
            &ctx,
        )
        .await;
    assert!(err_type.is_err());
    assert!(err_type.unwrap_err().to_string().contains("text"));

    // EvaluateJs missing code
    let err_eval = tool
        .execute(json!({ "action": "evaluate_js" }), &ctx)
        .await;
    assert!(err_eval.is_err());
    assert!(err_eval.unwrap_err().to_string().contains("code"));

    // Navigate missing URL
    let err_nav = tool.execute(json!({ "action": "navigate" }), &ctx).await;
    assert!(err_nav.is_err());
    assert!(err_nav.unwrap_err().to_string().contains("url"));
}

// ============================================================================
// 9. Mock CDP HTTP Management Server Test
// ============================================================================

#[tokio::test]
async fn test_mock_cdp_http_client() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("should bind local port");
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{}", addr);

    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let mut buf = [0u8; 1024];
            let n = stream.read(&mut buf).await.unwrap_or(0);
            let req_str = String::from_utf8_lossy(&buf[..n]);

            if req_str.starts_with("GET /json/version") {
                let body = json!({
                    "Browser": "Chrome/124.0.6367.60",
                    "Protocol-Version": "1.3",
                    "User-Agent": "Mozilla/5.0 Test",
                    "V8-Version": "12.4.254.14",
                    "webSocketDebuggerUrl": format!("ws://{}/devtools/browser/xyz", addr)
                })
                .to_string();

                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(resp.as_bytes()).await;
            } else if req_str.starts_with("GET /json/list") || req_str.starts_with("GET /json") {
                let body = json!([
                    {
                        "id": "PAGE_001",
                        "type": "page",
                        "title": "Fusion Home",
                        "url": "https://fusion.ai",
                        "webSocketDebuggerUrl": format!("ws://{}/devtools/page/PAGE_001", addr)
                    }
                ])
                .to_string();

                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(resp.as_bytes()).await;
            } else {
                let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\n\r\n").await;
            }
        }
    });

    let client = CdpClient::new(&base_url);
    assert!(client.is_online().await, "Mock CDP should be online");

    let ver = client.version().await.expect("version query should succeed");
    assert_eq!(ver.browser, "Chrome/124.0.6367.60");
    assert_eq!(ver.protocol_version, "1.3");

    let tabs = client.list_tabs().await.expect("list_tabs query should succeed");
    assert_eq!(tabs.len(), 1);
    assert_eq!(tabs[0].id, "PAGE_001");
    assert_eq!(tabs[0].title, "Fusion Home");
    assert_eq!(tabs[0].url, "https://fusion.ai");
}

// ============================================================================
// 10. Mock CDP WebSocket Handshake & JSON-RPC Test
// ============================================================================

#[tokio::test]
async fn test_mock_cdp_websocket_handshake_and_rpc() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("should bind local websocket port");
    let addr = listener.local_addr().unwrap();
    let ws_url = format!("ws://{}/devtools/page/TEST_TAB", addr);

    tokio::spawn(async move {
        if let Ok((mut stream, _)) = listener.accept().await {
            // Read handshake
            let mut buf = [0u8; 1024];
            let n = stream.read(&mut buf).await.unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]);
            assert!(req.contains("Upgrade: websocket"));

            // Send 101 Switching Protocols
            let response = "HTTP/1.1 101 Switching Protocols\r\n\
                            Upgrade: websocket\r\n\
                            Connection: Upgrade\r\n\
                            Sec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=\r\n\
                            \r\n";
            stream.write_all(response.as_bytes()).await.unwrap();

            // Read client JSON-RPC frame
            let mut header = [0u8; 2];
            stream.read_exact(&mut header).await.unwrap();
            let len = (header[1] & 0x7f) as usize;
            let mut mask = [0u8; 4];
            stream.read_exact(&mut mask).await.unwrap();
            let mut payload = vec![0u8; len];
            stream.read_exact(&mut payload).await.unwrap();
            for (i, b) in payload.iter_mut().enumerate() {
                *b ^= mask[i % 4];
            }
            let req_json: serde_json::Value =
                serde_json::from_slice(&payload).expect("should be JSON-RPC");
            let id = req_json.get("id").and_then(|v| v.as_u64()).unwrap_or(1);

            // Send unmasked server JSON-RPC response
            let resp_payload = json!({
                "id": id,
                "result": {
                    "result": {
                        "type": "string",
                        "value": "42"
                    }
                }
            })
            .to_string();
            let resp_bytes = resp_payload.as_bytes();

            let mut resp_frame = Vec::new();
            resp_frame.push(0x81); // FIN=1, Text=1
            resp_frame.push(resp_bytes.len() as u8); // Unmasked length < 126
            resp_frame.extend_from_slice(resp_bytes);

            stream.write_all(&resp_frame).await.unwrap();
            stream.flush().await.unwrap();
        }
    });

    let mut ws = CdpWebSocket::connect(&ws_url)
        .await
        .expect("WebSocket connection should succeed");

    let res = ws
        .call_method(
            "Runtime.evaluate",
            json!({ "expression": "6 * 7", "returnByValue": true }),
        )
        .await
        .expect("call_method should succeed");

    let val = res
        .get("result")
        .and_then(|r| r.get("value"))
        .and_then(|v| v.as_str())
        .expect("value should be present");

    assert_eq!(val, "42");
}
