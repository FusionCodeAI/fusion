use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;

use fusion::tools::file::{html_to_markdown, strip_html_blocks, FileReadTool};
use fusion::tools::types::{Tool, ToolContext};
use fusion::tools::uri_router::{is_http_url, resolve_internal_uri};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

/// Lightweight mock HTTP server using Tokio TcpListener.
struct MockHttpServer {
    addr: SocketAddr,
    shutdown_tx: Option<oneshot::Sender<()>>,
}

impl MockHttpServer {
    async fn start(status: u16, content_type: &str, body: &'static str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind tcp listener");
        let addr = listener.local_addr().expect("local addr");
        let (shutdown_tx, mut shutdown_rx) = oneshot::channel();

        let status_text = match status {
            200 => "OK",
            404 => "Not Found",
            500 => "Internal Server Error",
            _ => "Status",
        };

        let response = format!(
            "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            status,
            status_text,
            content_type,
            body.len(),
            body
        );

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => break,
                    accept_res = listener.accept() => {
                        if let Ok((mut socket, _)) = accept_res {
                            let resp = response.clone();
                            tokio::spawn(async move {
                                let mut buf = [0u8; 1024];
                                let _ = socket.read(&mut buf).await;
                                let _ = socket.write_all(resp.as_bytes()).await;
                                let _ = socket.shutdown().await;
                            });
                        }
                    }
                }
            }
        });

        Self {
            addr,
            shutdown_tx: Some(shutdown_tx),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }
}

impl Drop for MockHttpServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

fn test_context() -> ToolContext {
    ToolContext {
        cwd: PathBuf::from("/tmp"),
        env: HashMap::new(),
    }
}

#[tokio::test]
async fn test_url_reading_200_ok_sanitization() {
    let html = r#"<!DOCTYPE html>
<html>
<head>
    <title>Sample Site</title>
    <style>
        body { background: #333; color: #fff; }
        .hidden { display: none; }
    </style>
    <script type="text/javascript">
        console.log("tracking_token_12345");
    </script>
</head>
<body>
    <header class="site-header">
        <h1>Site Banner Header</h1>
        <p>Ignored header text</p>
    </header>

    <nav aria-label="Main Navigation">
        <ul>
            <li><a href="/home">Home</a></li>
            <li><a href="/about">About Us</a></li>
        </ul>
    </nav>

    <main>
        <h1>Welcome to Fusion</h1>
        <p>This is the first paragraph with <a href="https://example.com/docs">documentation link</a> and some useful text.</p>
        <p>Second paragraph with a<br>line break right here.</p>
        <h2>Features List</h2>
        <ul>
            <li>Lightning Fast Engine</li>
            <li>Subagent Collaboration</li>
        </ul>
    </main>

    <footer role="contentinfo">
        <p>&copy; 2026 Fusion Authors. All rights reserved.</p>
    </footer>
</body>
</html>"#;

    let server = MockHttpServer::start(200, "text/html; charset=utf-8", html).await;
    let tool = FileReadTool::new();
    let ctx = test_context();

    let target_url = server.url("/index.html");
    let result = tool
        .execute(json!({ "path": target_url }), &ctx)
        .await
        .expect("execution should succeed");

    // Verify main content and converted markdown
    assert!(result.contains("# Welcome to Fusion"), "Missing H1: {result}");
    assert!(
        result.contains("This is the first paragraph with [documentation link](https://example.com/docs) and some useful text."),
        "Missing paragraph with link: {result}"
    );
    assert!(result.contains("Second paragraph with a\nline break right here."), "Missing line break: {result}");
    assert!(result.contains("## Features List"), "Missing H2: {result}");
    assert!(result.contains("- Lightning Fast Engine"), "Missing list item 1: {result}");
    assert!(result.contains("- Subagent Collaboration"), "Missing list item 2: {result}");

    // Verify stripped blocks
    assert!(!result.contains("Site Banner Header"), "Header block was not stripped: {result}");
    assert!(!result.contains("Ignored header text"), "Header block text was not stripped: {result}");
    assert!(!result.contains("Main Navigation"), "Nav attribute was not stripped: {result}");
    assert!(!result.contains("About Us"), "Nav links were not stripped: {result}");
    assert!(!result.contains("tracking_token_12345"), "Script block was not stripped: {result}");
    assert!(!result.contains("background: #333"), "Style block was not stripped: {result}");
    assert!(!result.contains("Fusion Authors"), "Footer block was not stripped: {result}");
}

#[test]
fn test_html_to_markdown_unit_tags_and_entities() {
    let raw_html = r##"
        <header><h1>Ignored Header</h1></header>
        <nav><p>Ignored Nav</p></nav>
        <script>var x = 42;</script>
        <style>.x { color: blue; }</style>
        <h1>Heading 1</h1>
        <h2>Heading 2</h2>
        <h3>Heading 3</h3>
        <h4>Heading 4</h4>
        <h5>Heading 5</h5>
        <h6>Heading 6</h6>
        <p>Paragraph with &amp; &lt;brackets&gt; &quot;quotes&quot; and &#39;apostrophe&#39;.</p>
        <p>Break<br>line</p>
        <p><a href="https://fusion.ai">Fusion Home</a></p>
        <p><a href="#anchor">Internal Anchor</a></p>
        <ul>
            <li>One</li>
            <li>Two</li>
        </ul>
        <footer><p>Ignored Footer</p></footer>
    "##;

    let md = html_to_markdown(raw_html);

    assert!(!md.contains("Ignored Header"));
    assert!(!md.contains("Ignored Nav"));
    assert!(!md.contains("var x = 42;"));
    assert!(!md.contains(".x { color: blue; }"));
    assert!(!md.contains("Ignored Footer"));

    assert!(md.contains("# Heading 1"));
    assert!(md.contains("## Heading 2"));
    assert!(md.contains("### Heading 3"));
    assert!(md.contains("#### Heading 4"));
    assert!(md.contains("##### Heading 5"));
    assert!(md.contains("###### Heading 6"));
    assert!(md.contains("Paragraph with & <brackets> \"quotes\" and 'apostrophe'."));
    assert!(md.contains("Break\nline"));
    assert!(md.contains("[Fusion Home](https://fusion.ai)"));
    assert!(md.contains("Internal Anchor"));
    assert!(!md.contains("[Internal Anchor](#anchor)"));
    assert!(md.contains("- One"));
    assert!(md.contains("- Two"));
}

#[test]
fn test_strip_html_blocks_nested_and_comments() {
    let input = r#"
        <!-- Header comment -->
        <header class="main-header">
            <nav>Inner nav</nav>
            <h1>Header Title</h1>
        </header>
        <main>
            <p>Main content <!-- inline comment --> preserved.</p>
        </main>
        <footer id="f">
            <span>Footer span</span>
        </footer>
    "#;

    let stripped = strip_html_blocks(input);
    assert!(!stripped.contains("main-header"));
    assert!(!stripped.contains("Inner nav"));
    assert!(!stripped.contains("Header Title"));
    assert!(!stripped.contains("Footer span"));
    assert!(!stripped.contains("Header comment"));
    assert!(stripped.contains("Main content"));
    assert!(stripped.contains("preserved."));
}

#[tokio::test]
async fn test_url_reading_404_error() {
    let server = MockHttpServer::start(404, "text/plain", "Not Found").await;
    let tool = FileReadTool::new();
    let ctx = test_context();

    let target_url = server.url("/missing-page");
    let result = tool
        .execute(json!({ "path": target_url }), &ctx)
        .await
        .expect("should return Ok string with error description");

    assert!(
        result.contains("HTTP Error 404"),
        "Expected HTTP Error 404 in: {result}"
    );
    assert!(
        result.contains("Failed to read URL"),
        "Expected Failed to read URL in: {result}"
    );
    assert!(result.contains(&server.url("/missing-page")));
}

#[tokio::test]
async fn test_url_reading_500_error() {
    let server = MockHttpServer::start(500, "text/plain", "Server Error").await;
    let tool = FileReadTool::new();
    let ctx = test_context();

    let target_url = server.url("/crash");
    let result = tool
        .execute(json!({ "path": target_url }), &ctx)
        .await
        .expect("should return Ok string with error description");

    assert!(
        result.contains("HTTP Error 500"),
        "Expected HTTP Error 500 in: {result}"
    );
    assert!(
        result.contains("Failed to read URL"),
        "Expected Failed to read URL in: {result}"
    );
}

#[tokio::test]
async fn test_url_reading_connection_failure() {
    let tool = FileReadTool::new();
    let ctx = test_context();

    // Port 1 is reserved and not listening
    let unreachable_url = "http://127.0.0.1:1/test";
    let res = tool.execute(json!({ "path": unreachable_url }), &ctx).await;

    assert!(res.is_err(), "Expected connection error for unreachable port");
    let err_str = res.err().unwrap().to_string();
    assert!(
        err_str.contains("Failed to read URL"),
        "Expected 'Failed to read URL' error message: {err_str}"
    );
}

#[tokio::test]
async fn test_url_reading_raw_mode() {
    let html = "<html><head><script>alert(1);</script></head><body><p>Raw text</p></body></html>";
    let server = MockHttpServer::start(200, "text/html", html).await;
    let tool = FileReadTool::new();
    let ctx = test_context();

    let raw_url = format!("{}:raw", server.url("/page"));
    let result = tool
        .execute(json!({ "path": raw_url }), &ctx)
        .await
        .expect("raw read should succeed");

    assert_eq!(result, html, "Raw mode should return unmodified HTML");
}

#[test]
fn test_uri_router_url_handling() {
    // resolve_internal_uri must pass HTTP and HTTPS URLs through cleanly as None
    assert_eq!(resolve_internal_uri("http://example.com", None), None);
    assert_eq!(resolve_internal_uri("https://example.com/api", None), None);
    assert_eq!(resolve_internal_uri("http://localhost:8080/doc.html", None), None);

    // is_http_url checks
    assert!(is_http_url("http://example.com"));
    assert!(is_http_url("https://example.com"));
    assert!(is_http_url("  https://secure.example.com  "));
    assert!(!is_http_url("skill://brainstorming"));
    assert!(!is_http_url("rule://formatting"));
    assert!(!is_http_url("src/tools/file.rs"));
    assert!(!is_http_url("/tmp/test.txt"));
}
