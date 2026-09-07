use fusion::tools::lsp::client::{decode_message, encode_message, path_to_uri, LspClient};
use serde_json::json;
use std::path::Path;

#[test]
fn test_encode_message_format() {
    let payload = json!({
        "jsonrpc": "2.0",
        "method": "test/ping",
        "params": {
            "key": "value",
            "count": 42
        }
    });

    let encoded = encode_message(&payload);
    let encoded_str = std::str::from_utf8(&encoded).expect("valid utf-8");

    // Header must start with Content-Length: <len>\r\n\r\n
    assert!(encoded_str.starts_with("Content-Length: "));
    assert!(encoded_str.contains("\r\n\r\n"));

    let parts: Vec<&str> = encoded_str.split("\r\n\r\n").collect();
    assert_eq!(parts.len(), 2, "must have header and body separated by \\r\\n\\r\\n");

    let header_line = parts[0];
    let body_str = parts[1];

    let content_len: usize = header_line
        .strip_prefix("Content-Length: ")
        .expect("starts with Content-Length: ")
        .parse()
        .expect("valid integer");

    assert_eq!(content_len, body_str.as_bytes().len());

    let parsed_body: serde_json::Value = serde_json::from_str(body_str).expect("valid JSON body");
    assert_eq!(parsed_body, payload);
}

#[test]
fn test_decode_message_complete_frame() {
    let payload = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": {
            "capabilities": {
                "definitionProvider": true
            }
        }
    });

    let mut buf = encode_message(&payload);
    assert!(!buf.is_empty());

    let decoded = decode_message(&mut buf).expect("should decode complete message");
    assert_eq!(decoded, payload);
    assert!(buf.is_empty(), "buffer should be fully drained after complete frame");
}

#[test]
fn test_decode_message_partial_buffer_headers() {
    let mut partial_buf = b"Content-Length: 42\r\n".to_vec();
    let original_len = partial_buf.len();

    let result = decode_message(&mut partial_buf);
    assert!(result.is_none(), "partial header should return None");
    assert_eq!(partial_buf.len(), original_len, "buffer must not be modified");
}

#[test]
fn test_decode_message_partial_buffer_body() {
    let payload = json!({
        "jsonrpc": "2.0",
        "method": "textDocument/publishDiagnostics",
        "params": {
            "uri": "file:///test.rs",
            "diagnostics": [
                {
                    "message": "unused variable",
                    "severity": 2
                }
            ]
        }
    });

    let full_encoded = encode_message(&payload);
    assert!(full_encoded.len() > 10);

    // Provide all headers and only partial body
    let cut_point = full_encoded.len() - 5;
    let mut partial_buf = full_encoded[..cut_point].to_vec();

    let result = decode_message(&mut partial_buf);
    assert!(result.is_none(), "incomplete body should return None");
    assert_eq!(
        partial_buf.len(),
        cut_point,
        "buffer must not be modified when body is incomplete"
    );

    // Now append the missing 5 bytes
    partial_buf.extend_from_slice(&full_encoded[cut_point..]);

    let decoded = decode_message(&mut partial_buf).expect("should decode once complete");
    assert_eq!(decoded, payload);
    assert!(partial_buf.is_empty(), "buffer should be completely drained");
}

#[test]
fn test_decode_message_multiple_frames() {
    let msg1 = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": "first"
    });
    let msg2 = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "result": "second"
    });

    let mut buf = Vec::new();
    buf.extend_from_slice(&encode_message(&msg1));
    buf.extend_from_slice(&encode_message(&msg2));

    let decoded1 = decode_message(&mut buf).expect("should decode first message");
    assert_eq!(decoded1, msg1);

    let decoded2 = decode_message(&mut buf).expect("should decode second message");
    assert_eq!(decoded2, msg2);

    assert!(buf.is_empty(), "buffer should be empty after both messages");
    assert!(decode_message(&mut buf).is_none(), "empty buffer yields None");
}

#[test]
fn test_decode_message_extra_headers() {
    let payload = json!({ "jsonrpc": "2.0", "result": 123 });
    let body = serde_json::to_string(&payload).unwrap();
    let raw = format!(
        "Content-Type: application/vscode-jsonrpc; charset=utf-8\r\nContent-Length: {}\r\n\r\n{}",
        body.len(),
        body
    );

    let mut buf = raw.into_bytes();
    let decoded = decode_message(&mut buf).expect("should handle extra headers");
    assert_eq!(decoded, payload);
    assert!(buf.is_empty());
}

#[test]
fn test_path_to_uri() {
    let path = Path::new("/workspace/project/src/main.rs");
    let uri = path_to_uri(path);
    assert!(uri.starts_with("file://"));
    assert!(uri.contains("src/main.rs"));
}

#[tokio::test]
async fn test_lsp_client_spawn_mock_process() {
    // Test creating LspClient with a standard command (cat on unix)
    #[cfg(unix)]
    {
        let client = LspClient::new("cat", &[] as &[&str]);
        assert!(client.is_ok(), "spawning cat should succeed on unix");
        let client = client.unwrap();

        // Shutdown cat
        let shutdown_res = client.shutdown().await;
        assert!(shutdown_res.is_ok());
    }
}
