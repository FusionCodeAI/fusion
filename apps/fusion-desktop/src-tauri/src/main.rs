// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesktopSessionSummary {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    pub model: String,
    pub message_count: usize,
    pub preview: String,
}

fn fusion_sessions_dir() -> PathBuf {
    if let Ok(custom) = std::env::var("FUSION_SESSIONS_DIR") {
        return PathBuf::from(custom);
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".fusion").join("sessions");
    }
    if let Ok(userprofile) = std::env::var("USERPROFILE") {
        return PathBuf::from(userprofile).join(".fusion").join("sessions");
    }
    PathBuf::from(".fusion").join("sessions")
}

#[tauri::command]
fn list_fusion_sessions() -> Result<Vec<DesktopSessionSummary>, String> {
    let dir = fusion_sessions_dir();
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let entries = fs::read_dir(&dir).map_err(|e| e.to_string())?;
    let mut summaries = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("json") {
            if let Ok(content) = fs::read_to_string(&path) {
                if let Ok(val) = serde_json::from_str::<Value>(&content) {
                    let id = val.get("id").and_then(|v| v.as_str()).unwrap_or_else(|| {
                        path.file_stem().and_then(|s| s.to_str()).unwrap_or("unknown")
                    }).to_string();

                    let title = val.get("title").and_then(|v| v.as_str()).unwrap_or("General chat conversation").to_string();
                    let created_at = val.get("created_at").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let updated_at = val.get("updated_at").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let model = val.get("active_model").and_then(|v| v.as_str()).unwrap_or("deepseek-v4-flash-0731").to_string();
                    let messages = val.get("messages").and_then(|v| v.as_array());
                    let message_count = messages.map(|m| m.len()).unwrap_or(0);

                    let preview = messages.and_then(|m| m.last())
                        .and_then(|last| last.get("content"))
                        .and_then(|c| c.as_str())
                        .map(|s| {
                            let clean = s.trim();
                            if clean.chars().count() > 100 {
                                let truncated: String = clean.chars().take(100).collect();
                                format!("{}...", truncated)
                            } else {
                                clean.to_string()
                            }
                        })
                        .unwrap_or_default();

                    summaries.push(DesktopSessionSummary {
                        id,
                        title,
                        created_at,
                        updated_at,
                        model,
                        message_count,
                        preview,
                    });
                }
            }
        }
    }

    // Sort newest first by updated_at
    summaries.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

    Ok(summaries)
}

#[tauri::command]
fn load_fusion_session(id: String) -> Result<Value, String> {
    let dir = fusion_sessions_dir();
    let direct_path = dir.join(format!("{}.json", id));

    let target_path = if direct_path.exists() {
        direct_path
    } else {
        let mut found = None;
        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                    if stem.starts_with(&id) && p.extension().and_then(|s| s.to_str()) == Some("json") {
                        found = Some(p);
                        break;
                    }
                }
            }
        }
        found.ok_or_else(|| format!("Session not found with ID or prefix '{}'", id))?
    };

    let content = fs::read_to_string(&target_path).map_err(|e| e.to_string())?;
    let val: Value = serde_json::from_str(&content).map_err(|e| e.to_string())?;
    Ok(val)
}

#[tauri::command]
fn save_fusion_session(session: Value) -> Result<String, String> {
    let dir = fusion_sessions_dir();
    if !dir.exists() {
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    }

    let id = session.get("id").and_then(|v| v.as_str()).ok_or_else(|| "Session missing 'id' field".to_string())?;
    let target_path = dir.join(format!("{}.json", id));
    let temp_path = dir.join(format!("{}.tmp.{}", id, std::process::id()));

    let content = serde_json::to_string_pretty(&session).map_err(|e| e.to_string())?;
    fs::write(&temp_path, &content).map_err(|e| e.to_string())?;
    fs::rename(&temp_path, &target_path).map_err(|e| e.to_string())?;

    Ok(id.to_string())
}

#[allow(dead_code)]
#[tauri::command]
fn delete_fusion_session(id: String) -> Result<bool, String> {
    let dir = fusion_sessions_dir();
    let path = dir.join(format!("{}.json", id));
    if path.exists() {
        fs::remove_file(path).map_err(|e| e.to_string())?;
        Ok(true)
    } else {
        Ok(false)
    }
}

#[tauri::command]
async fn pick_project_folder() -> Result<Option<String>, String> {
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("osascript")
            .arg("-e")
            .arg("try\nPOSIX path of (choose folder with prompt \"Select Project Folder\")\non error\n\"\"\nend try")
            .output()
            .map_err(|e| e.to_string())?;

        let res = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if res.is_empty() {
            Ok(None)
        } else {
            Ok(Some(res))
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok(None)
    }
}

fn find_fusion_binary() -> Result<PathBuf, String> {
    if let Ok(custom) = std::env::var("FUSION_BINARY_PATH") {
        let p = PathBuf::from(custom);
        if p.exists() {
            return Ok(p);
        }
    }

    if let Ok(home) = std::env::var("HOME") {
        let p = PathBuf::from(home).join(".local").join("bin").join("fusion");
        if p.exists() {
            return Ok(p);
        }
    }

    if let Ok(current_exe) = std::env::current_exe() {
        let mut cur = current_exe.as_path();
        while let Some(parent) = cur.parent() {
            let rel = parent.join("target").join("release").join("fusion");
            if rel.exists() {
                return Ok(rel);
            }
            let deb = parent.join("target").join("debug").join("fusion");
            if deb.exists() {
                return Ok(deb);
            }
            cur = parent;
        }
    }

    let candidates = [
        PathBuf::from("../../target/release/fusion"),
        PathBuf::from("../../../target/release/fusion"),
        PathBuf::from("../../../../target/release/fusion"),
        PathBuf::from("target/release/fusion"),
    ];

    for c in &candidates {
        if c.exists() {
            if let Ok(canon) = c.canonicalize() {
                return Ok(canon);
            }
            return Ok(c.clone());
        }
    }

    if let Ok(output) = std::process::Command::new("which").arg("fusion").output() {
        if output.status.success() {
            let path_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path_str.is_empty() {
                let p = PathBuf::from(path_str);
                if p.exists() {
                    return Ok(p);
                }
            }
        }
    }

    Err("Could not locate 'fusion' binary. Please build it with 'cargo build --release' or set FUSION_BINARY_PATH.".to_string())
}

fn clean_terminal_output(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&']') {
                chars.next();
                for sc in chars.by_ref() {
                    if sc == '\x07' || sc == '\n' {
                        break;
                    }
                }
            } else if chars.peek() == Some(&'[') {
                chars.next();
                for sc in chars.by_ref() {
                    if sc.is_alphabetic() {
                        break;
                    }
                }
            }
        } else {
            out.push(c);
        }
    }
    out.trim().to_string()
}

#[tauri::command]
async fn execute_fusion_turn(
    prompt: String,
    model: Option<String>,
    session_id: Option<String>,
    cwd: Option<String>,
) -> Result<String, String> {
    let binary = find_fusion_binary()?;
    let mut cmd = std::process::Command::new(&binary);

    if let Some(m) = model {
        let clean_m = m.trim();
        if !clean_m.is_empty() {
            cmd.arg("-m").arg(clean_m);
        }
    }

    if let Some(s) = session_id {
        let clean_s = s.trim();
        if !clean_s.is_empty() {
            let direct = fusion_sessions_dir().join(format!("{}.json", clean_s));
            if direct.exists() {
                cmd.arg("-r").arg(clean_s);
            }
        }
    }

    if let Some(dir) = cwd {
        let clean_d = dir.trim();
        if !clean_d.is_empty() {
            cmd.arg("-C").arg(clean_d);
        }
    }

    cmd.arg(&prompt);
    cmd.envs(std::env::vars());

    let output = cmd.output().map_err(|e| format!("Failed to spawn {}: {}", binary.display(), e))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    let cleaned_stdout = clean_terminal_output(&stdout);

    if output.status.success() {
        Ok(cleaned_stdout)
    } else if !cleaned_stdout.is_empty() {
        Ok(cleaned_stdout)
    } else {
        Err(if !stderr.trim().is_empty() { stderr } else { format!("Process exited with status {}", output.status) })
    }
}

#[tauri::command]
async fn stream_fusion_acp(
    prompt: String,
    model: Option<String>,
    session_id: Option<String>,
    cwd: Option<String>,
    on_event: tauri::ipc::Channel<Value>,
) -> Result<String, String> {
    let binary = find_fusion_binary()?;
    let mut cmd = tokio::process::Command::new(&binary);
    cmd.arg("--acp");

    if let Some(dir) = cwd {
        let clean_d = dir.trim();
        if !clean_d.is_empty() {
            cmd.arg("-C").arg(clean_d);
        }
    }

    cmd.stdin(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::null());
    cmd.envs(std::env::vars());

    let mut child = cmd.spawn().map_err(|e| format!("Failed to spawn {}: {}", binary.display(), e))?;

    let mut stdin = child.stdin.take().ok_or_else(|| "Failed to open stdin".to_string())?;
    let stdout = child.stdout.take().ok_or_else(|| "Failed to open stdout".to_string())?;

    let init_req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": 1,
            "clientInfo": { "name": "fusion-desktop", "version": "2.0.0" },
            "capabilities": {}
        }
    });
    stdin.write_all(format!("{}\n", init_req).as_bytes()).await.map_err(|e| e.to_string())?;
    stdin.flush().await.map_err(|e| e.to_string())?;

    let mut reader = tokio::io::BufReader::new(stdout).lines();
    let _ = reader.next_line().await.map_err(|e| e.to_string())?;

    let target_session_id = session_id.unwrap_or_default();
    let direct_path = fusion_sessions_dir().join(format!("{}.json", target_session_id));

    let actual_session_id = if !target_session_id.is_empty() && direct_path.exists() {
        let load_req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "session/load",
            "params": { "sessionId": target_session_id }
        });
        stdin.write_all(format!("{}\n", load_req).as_bytes()).await.map_err(|e| e.to_string())?;
        stdin.flush().await.map_err(|e| e.to_string())?;
        let _ = reader.next_line().await;
        target_session_id
    } else {
        let chosen_model = model.unwrap_or_else(|| "deepseek-v4-flash-0731".to_string());
        let new_req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "session/new",
            "params": { "model": chosen_model }
        });
        stdin.write_all(format!("{}\n", new_req).as_bytes()).await.map_err(|e| e.to_string())?;
        stdin.flush().await.map_err(|e| e.to_string())?;
        let new_line = reader.next_line().await.map_err(|e| e.to_string())?.unwrap_or_default();
        let val: Value = serde_json::from_str(&new_line).unwrap_or_default();
        val.get("result").and_then(|r| r.get("sessionId")).and_then(|s| s.as_str()).unwrap_or("").to_string()
    };

    let prompt_req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "session/prompt",
        "params": {
            "sessionId": actual_session_id,
            "prompt": prompt
        }
    });
    stdin.write_all(format!("{}\n", prompt_req).as_bytes()).await.map_err(|e| e.to_string())?;
    stdin.flush().await.map_err(|e| e.to_string())?;

    let mut full_text = String::new();

    while let Ok(Some(line)) = reader.next_line().await {
        if let Ok(val) = serde_json::from_str::<Value>(&line) {
            if val.get("id") == Some(&serde_json::json!(3)) {
                break;
            }

            if val.get("method") == Some(&serde_json::json!("session/update")) {
                if let Some(params) = val.get("params") {
                    if let Some(update) = params.get("update") {
                        let kind = update.get("sessionUpdate").and_then(|k| k.as_str())
                            .or_else(|| update.get("kind").and_then(|k| k.as_str()))
                            .unwrap_or("");

                        if kind == "agent_message_chunk" {
                            if let Some(text_val) = update.get("content").and_then(|c| c.get("text")).and_then(|t| t.as_str()) {
                                full_text.push_str(text_val);
                                let _ = on_event.send(serde_json::json!({
                                    "type": "chunk",
                                    "text": text_val
                                }));
                            }
                        } else if kind == "agent_thought_chunk" || kind == "thought" {
                            let t_val = update.get("thought").and_then(|t| t.as_str())
                                .or_else(|| update.get("content").and_then(|c| c.get("text")).and_then(|t| t.as_str()));
                            if let Some(thought_text) = t_val {
                                let _ = on_event.send(serde_json::json!({
                                    "type": "thought",
                                    "text": thought_text
                                }));
                            }
                        } else if kind == "tool_call" {
                            let name = update.get("name").and_then(|t| t.as_str()).unwrap_or("tool");
                            let title = format!("Ran {}", name);
                            let _ = on_event.send(serde_json::json!({
                                "type": "step",
                                "title": title,
                                "status": "running"
                            }));
                        } else if kind == "tool_call_result" {
                            let name = update.get("name").and_then(|t| t.as_str()).unwrap_or("tool");
                            let title = format!("Completed {}", name);
                            let _ = on_event.send(serde_json::json!({
                                "type": "step",
                                "title": title,
                                "status": "completed"
                            }));
                        }
                    }
                }
            }
        }
    }

    let _ = child.kill().await;
    Ok(full_text)
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![
            list_fusion_sessions,
            load_fusion_session,
            save_fusion_session,
            delete_fusion_session,
            execute_fusion_turn,
            stream_fusion_acp,
            pick_project_folder,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
