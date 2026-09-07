use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tokio::sync::oneshot;

/// Converts a filesystem path into an LSP file:// URI string.
pub fn path_to_uri(path: &Path) -> String {
    let abs_path = if path.is_absolute() {
        path.to_path_buf()
    } else if let Ok(canon) = path.canonicalize() {
        canon
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("."))
            .join(path)
    };

    let s = abs_path.to_string_lossy();
    if s.starts_with('/') {
        format!("file://{}", s)
    } else {
        format!("file:///{}", s.replace('\\', "/"))
    }
}

/// Encodes a JSON-RPC message into LSP header framing:
/// `Content-Length: <len>\r\n\r\n<payload>`
pub fn encode_message(payload: &Value) -> Vec<u8> {
    let content = serde_json::to_vec(payload).unwrap_or_default();
    let header = format!("Content-Length: {}\r\n\r\n", content.len());
    let mut buf = Vec::with_capacity(header.len() + content.len());
    buf.extend_from_slice(header.as_bytes());
    buf.extend_from_slice(&content);
    buf
}

/// Decodes a framed LSP message from `buf`.
///
/// If a complete message is available:
/// - Drains the headers and payload bytes from `buf`.
/// - Returns `Some(Value)` containing the parsed JSON.
///
/// If the buffer does not yet contain a complete message or header delimiter:
/// - Leaves `buf` unmodified.
/// - Returns `None`.
pub fn decode_message(buf: &mut Vec<u8>) -> Option<Value> {
    let (header_len, body_offset) = if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
        (pos, pos + 4)
    } else if let Some(pos) = buf.windows(2).position(|w| w == b"\n\n") {
        (pos, pos + 2)
    } else {
        return None;
    };

    let header_str = std::str::from_utf8(&buf[..header_len]).ok()?;
    let mut content_length: Option<usize> = None;

    for line in header_str.lines() {
        let trimmed = line.trim();
        if let Some((name, val)) = trimmed.split_once(':') {
            if name.trim().eq_ignore_ascii_case("content-length") {
                if let Ok(len) = val.trim().parse::<usize>() {
                    content_length = Some(len);
                }
            }
        }
    }

    let content_length = content_length?;

    if buf.len() < body_offset + content_length {
        return None;
    }

    let body_bytes = &buf[body_offset..body_offset + content_length];
    let value = serde_json::from_slice::<Value>(body_bytes).ok();

    buf.drain(..body_offset + content_length);

    value
}

/// Pure-Rust asynchronous LSP client using `tokio::process::Command` over standard I/O pipes.
pub struct LspClient {
    next_id: AtomicI64,
    pending: Arc<Mutex<HashMap<i64, oneshot::Sender<Result<Value, String>>>>>,
    writer_tx: mpsc::Sender<Vec<u8>>,
    child: Arc<tokio::sync::Mutex<Child>>,
}

impl LspClient {
    /// Spawns an LSP server process and initializes I/O loops.
    pub fn new<I, S>(cmd: &str, args: I) -> anyhow::Result<Self>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        let mut child = Command::new(cmd)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("Failed to capture stdin of LSP child process"))?;
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("Failed to capture stdout of LSP child process"))?;

        // Drain stderr in background to prevent child process deadlocks
        if let Some(mut stderr) = child.stderr.take() {
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                while let Ok(n) = stderr.read(&mut buf).await {
                    if n == 0 {
                        break;
                    }
                }
            });
        }

        // Writer task
        let (writer_tx, mut writer_rx) = mpsc::channel::<Vec<u8>>(128);
        tokio::spawn(async move {
            while let Some(bytes) = writer_rx.recv().await {
                if stdin.write_all(&bytes).await.is_err() {
                    break;
                }
                if stdin.flush().await.is_err() {
                    break;
                }
            }
        });

        // Response tracking
        let pending = Arc::new(Mutex::new(HashMap::<i64, oneshot::Sender<Result<Value, String>>>::new()));
        let pending_clone = pending.clone();
        let writer_tx_clone = writer_tx.clone();

        // Reader task
        tokio::spawn(async move {
            let mut read_buf = Vec::new();
            let mut chunk = [0u8; 8192];
            loop {
                match stdout.read(&mut chunk).await {
                    Ok(0) => break,
                    Ok(n) => {
                        read_buf.extend_from_slice(&chunk[..n]);
                        while let Some(msg) = decode_message(&mut read_buf) {
                            Self::handle_message(&msg, &pending_clone, &writer_tx_clone);
                        }
                    }
                    Err(_) => break,
                }
            }

            // If stdout closed, cancel all remaining pending requests
            if let Ok(mut map) = pending_clone.lock() {
                for (_id, tx) in map.drain() {
                    let _ = tx.send(Err("LSP server closed connection".to_string()));
                }
            }
        });

        Ok(Self {
            next_id: AtomicI64::new(1),
            pending,
            writer_tx,
            child: Arc::new(tokio::sync::Mutex::new(child)),
        })
    }

    /// Asynchronous alias for `new`.
    pub async fn start<I, S>(cmd: &str, args: I) -> anyhow::Result<Self>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        Self::new(cmd, args)
    }

    fn handle_message(
        msg: &Value,
        pending: &Arc<Mutex<HashMap<i64, oneshot::Sender<Result<Value, String>>>>>,
        writer_tx: &mpsc::Sender<Vec<u8>>,
    ) {
        if let Some(id_val) = msg.get("id") {
            if let Some(id) = id_val.as_i64() {
                let sender = if let Ok(mut map) = pending.lock() {
                    map.remove(&id)
                } else {
                    None
                };

                if let Some(tx) = sender {
                    if let Some(error) = msg.get("error") {
                        let err_msg = error
                            .get("message")
                            .and_then(|m| m.as_str())
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| error.to_string());
                        let _ = tx.send(Err(err_msg));
                    } else {
                        let result = msg.get("result").cloned().unwrap_or(Value::Null);
                        let _ = tx.send(Ok(result));
                    }
                    return;
                }
            }

            // Server-to-client request: reply with null result so server does not hang
            if msg.get("method").and_then(|m| m.as_str()).is_some() {
                let resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id_val,
                    "result": null
                });
                let _ = writer_tx.try_send(encode_message(&resp));
            }
        }
    }

    /// Sends a JSON-RPC request with an incrementing integer ID and awaits the response.
    pub async fn send_request(&self, method: &str, params: Value) -> anyhow::Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();

        {
            let mut map = self
                .pending
                .lock()
                .map_err(|_| anyhow::anyhow!("Pending requests lock poisoned"))?;
            map.insert(id, tx);
        }

        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });

        let bytes = encode_message(&req);
        if let Err(e) = self.writer_tx.send(bytes).await {
            if let Ok(mut map) = self.pending.lock() {
                map.remove(&id);
            }
            anyhow::bail!("Failed to send LSP request: {}", e);
        }

        match rx.await {
            Ok(Ok(val)) => Ok(val),
            Ok(Err(err_msg)) => anyhow::bail!("LSP error for '{}': {}", method, err_msg),
            Err(_) => anyhow::bail!("LSP server dropped response channel for '{}'", method),
        }
    }

    /// Sends a JSON-RPC notification (no ID, no response awaited).
    pub async fn send_notification(&self, method: &str, params: Value) -> anyhow::Result<()> {
        let notif = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        let bytes = encode_message(&notif);
        self.writer_tx
            .send(bytes)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to send LSP notification: {}", e))?;
        Ok(())
    }

    /// Sends `initialize` request to the language server.
    pub async fn initialize(&self, root_path: &Path) -> anyhow::Result<Value> {
        let root_uri = path_to_uri(root_path);
        let root_path_str = root_path.to_str().unwrap_or("");
        let params = serde_json::json!({
            "processId": std::process::id(),
            "rootPath": root_path_str,
            "rootUri": root_uri,
            "capabilities": {
                "textDocument": {
                    "definition": { "dynamicRegistration": true },
                    "references": { "dynamicRegistration": true },
                    "synchronization": {
                        "didOpen": true,
                        "didChange": true,
                        "didClose": true
                    }
                },
                "workspace": {
                    "configuration": true
                }
            },
            "initializationOptions": null
        });

        self.send_request("initialize", params).await
    }

    /// Sends `initialized` notification to the language server.
    pub async fn initialized(&self) -> anyhow::Result<()> {
        self.send_notification("initialized", serde_json::json!({})).await
    }

    /// Sends `textDocument/didOpen` notification.
    pub async fn did_open(&self, path: &Path, language_id: &str, text: &str) -> anyhow::Result<()> {
        let uri = path_to_uri(path);
        let params = serde_json::json!({
            "textDocument": {
                "uri": uri,
                "languageId": language_id,
                "version": 1,
                "text": text
            }
        });
        self.send_notification("textDocument/didOpen", params).await
    }

    /// Sends `textDocument/definition` request and returns matching location entries.
    pub async fn goto_definition(&self, path: &Path, line: u32, character: u32) -> anyhow::Result<Vec<Value>> {
        let uri = path_to_uri(path);
        let params = serde_json::json!({
            "textDocument": {
                "uri": uri
            },
            "position": {
                "line": line,
                "character": character
            }
        });

        let res = self.send_request("textDocument/definition", params).await?;
        if res.is_null() {
            Ok(Vec::new())
        } else if let Some(arr) = res.as_array() {
            Ok(arr.clone())
        } else if res.is_object() {
            Ok(vec![res])
        } else {
            Ok(Vec::new())
        }
    }

    /// Sends `textDocument/references` request and returns reference location entries.
    pub async fn find_references(&self, path: &Path, line: u32, character: u32) -> anyhow::Result<Vec<Value>> {
        let uri = path_to_uri(path);
        let params = serde_json::json!({
            "textDocument": {
                "uri": uri
            },
            "position": {
                "line": line,
                "character": character
            },
            "context": {
                "includeDeclaration": true
            }
        });

        let res = self.send_request("textDocument/references", params).await?;
        if res.is_null() {
            Ok(Vec::new())
        } else if let Some(arr) = res.as_array() {
            Ok(arr.clone())
        } else if res.is_object() {
            Ok(vec![res])
        } else {
            Ok(Vec::new())
        }
    }

    /// Gracefully shuts down the LSP server:
    /// Sends `shutdown` request, `exit` notification, and waits for process exit.
    pub async fn shutdown(&self) -> anyhow::Result<()> {
        let _ = self.send_request("shutdown", Value::Null).await;
        let _ = self.send_notification("exit", Value::Null).await;

        let mut child = self.child.lock().await;
        match tokio::time::timeout(tokio::time::Duration::from_millis(1500), child.wait()).await {
            Ok(_) => Ok(()),
            Err(_) => {
                let _ = child.kill().await;
                Ok(())
            }
        }
    }
}
