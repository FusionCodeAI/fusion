// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

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
        // Try finding by prefix
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

fn find_fusion_binary() -> Result<PathBuf, String> {
    if let Ok(custom) = std::env::var("FUSION_BINARY_PATH") {
        let p = PathBuf::from(custom);
        if p.exists() {
            return Ok(p);
        }
    }

    if let Ok(current_exe) = std::env::current_exe() {
        if let Some(parent) = current_exe.parent() {
            let adjacent = parent.join("fusion");
            if adjacent.exists() {
                return Ok(adjacent);
            }
            let bin_sub = parent.join("bin").join("fusion");
            if bin_sub.exists() {
                return Ok(bin_sub);
            }
            let resources_bin = parent.join("../Resources/bin/fusion");
            if resources_bin.exists() {
                return Ok(resources_bin);
            }
        }
    }

    let candidates = [
        PathBuf::from("../../target/release/fusion"),
        PathBuf::from("../../target/debug/fusion"),
        PathBuf::from("../../../target/release/fusion"),
        PathBuf::from("../../../target/debug/fusion"),
        PathBuf::from("target/release/fusion"),
        PathBuf::from("target/debug/fusion"),
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
        if !m.trim().is_empty() {
            cmd.arg("-m").arg(m);
        }
    }

    if let Some(s) = session_id {
        if !s.trim().is_empty() {
            cmd.arg("-r").arg(s);
        }
    }

    if let Some(dir) = cwd {
        if !dir.trim().is_empty() {
            cmd.arg("-C").arg(dir);
        }
    }

    cmd.arg(&prompt);
    cmd.envs(std::env::vars());

    let output = cmd.output().map_err(|e| format!("Failed to spawn {}: {}", binary.display(), e))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if output.status.success() {
        Ok(stdout)
    } else {
        Err(if !stderr.trim().is_empty() { stderr } else { format!("Process exited with status {}", output.status) })
    }
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
