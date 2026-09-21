// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#[cfg(target_os = "macos")]
mod macos_notification;


use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use tauri::{Emitter, Manager};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

fn notification_response_opens_session(response: &notify_rust::NotificationResponse) -> bool {
    matches!(
        response,
        notify_rust::NotificationResponse::Default | notify_rust::NotificationResponse::Action(_)
    )
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesktopSessionSummary {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    pub model: String,
    pub message_count: usize,
    pub preview: String,
    pub workspace: Option<String>,
    pub workspace_name: Option<String>,
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
                    let created_at = val.get("created_at").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let id = val.get("id").and_then(|v| v.as_str()).unwrap_or_else(|| {
                        path.file_stem().and_then(|s| s.to_str()).unwrap_or("unknown")
                    }).to_string();
                    let messages = val.get("messages").and_then(|v| v.as_array());
                    let raw_title = val.get("title").and_then(|v| v.as_str()).unwrap_or("").trim();
                    let title = if !raw_title.is_empty() && raw_title != "General chat conversation" {
                        raw_title.to_string()
                    } else {
                        // Auto-derive title from first user message
                        let mut derived = None;
                        if let Some(msgs) = messages {
                            for m in msgs {
                                if m.get("role").and_then(|r| r.as_str()) == Some("user") {
                                    if let Some(c) = m.get("content").and_then(|c| c.as_str()) {
                                        let clean = c.trim();
                                        if !clean.is_empty() {
                                            let first_line = clean.lines().next().unwrap_or(clean);
                                            let sentence = first_line.split(&['.', '?', '!'][..]).next().unwrap_or(first_line).trim();
                                            let candidate = if sentence.is_empty() { first_line } else { sentence };
                                            if candidate.chars().count() > 48 {
                                                let mut tr: String = candidate.chars().take(46).collect();
                                                tr.push_str("...");
                                                derived = Some(tr);
                                            } else {
                                                derived = Some(candidate.to_string());
                                            }
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                        derived.unwrap_or_else(|| "New Conversation".to_string())
                    };
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
                    let workspace = val.get("workspace").and_then(|w| w.as_str()).map(|s| s.to_string());
                    let workspace_name = workspace.as_ref().and_then(|w| {
                        PathBuf::from(w).file_name().and_then(|n| n.to_str()).map(|s| s.to_string())
                    });

                    summaries.push(DesktopSessionSummary {
                        id,
                        title,
                        created_at,
                        updated_at,
                        model,
                        message_count,
                        preview,
                        workspace,
                        workspace_name,
                    });
                }
            }
        } else if path.is_dir() {
            let dir_name = entry.file_name().to_string_lossy().to_string();
            if dir_name.starts_with("%2F") {
                let decoded_ws = dir_name.replace("%2F", "/").replace("%20", " ");
                let ws_name = PathBuf::from(&decoded_ws)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("Workspace")
                    .to_string();

                if let Ok(sub_entries) = fs::read_dir(&path) {
                    for sub in sub_entries.flatten() {
                        let sub_p = sub.path();
                        if sub_p.is_dir() {
                            let summary_path = sub_p.join("summary.json");
                            if summary_path.exists() {
                                if let Ok(c) = fs::read_to_string(&summary_path) {
                                    if let Ok(sum) = serde_json::from_str::<Value>(&c) {
                                        let sid = sum.get("info").and_then(|i| i.get("id")).and_then(|v| v.as_str()).map(|s| s.to_string())
                                            .unwrap_or_else(|| sub.file_name().to_string_lossy().to_string());
                                        let raw_summary = sum.get("session_summary").and_then(|s| s.as_str()).unwrap_or("");
                                        let title = if !raw_summary.trim().is_empty() {
                                            raw_summary.to_string()
                                        } else {
                                            format!("{} session", ws_name)
                                        };
                                        let created_at = sum.get("created_at").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                        let updated_at = sum.get("updated_at").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                        let model = sum.get("current_model_id").and_then(|v| v.as_str()).unwrap_or("deepseek-v4-flash-0731").to_string();
                                        let message_count = sum.get("num_chat_messages").or_else(|| sum.get("num_messages")).and_then(|m| m.as_u64()).unwrap_or(0) as usize;

                                        summaries.push(DesktopSessionSummary {
                                            id: sid,
                                            title,
                                            created_at,
                                            updated_at,
                                            model,
                                            message_count,
                                            preview: format!("Workspace session in {}", ws_name),
                                            workspace: Some(decoded_ws.clone()),
                                            workspace_name: Some(ws_name.clone()),
                                        });
                                    }
                                }
                            }
                        }
                    }
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

    if direct_path.exists() {
        let content = fs::read_to_string(&direct_path).map_err(|e| e.to_string())?;
        return serde_json::from_str(&content).map_err(|e| e.to_string());
    }

    // Check root files by prefix
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                if stem.starts_with(&id) && p.extension().and_then(|s| s.to_str()) == Some("json") {
                    let content = fs::read_to_string(&p).map_err(|e| e.to_string())?;
                    return serde_json::from_str(&content).map_err(|e| e.to_string());
                }
            }
        }
    }

    // Check workspace subdirectories
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                let session_dir = p.join(&id);
                if session_dir.is_dir() {
                    let summary_p = session_dir.join("summary.json");
                    if summary_p.exists() {
                        let c = fs::read_to_string(&summary_p).map_err(|e| e.to_string())?;
                        let mut sum_val: Value = serde_json::from_str(&c).map_err(|e| e.to_string())?;
                        let mut msgs = Vec::new();

                        let history_p = session_dir.join("chat_history.jsonl");
                        if history_p.exists() {
                            if let Ok(h_content) = fs::read_to_string(&history_p) {
                                for line in h_content.lines() {
                                    if let Ok(l_val) = serde_json::from_str::<Value>(line) {
                                        let role = l_val.get("type").and_then(|t| t.as_str()).unwrap_or("user");
                                        let text = if let Some(c_arr) = l_val.get("content").and_then(|c| c.as_array()) {
                                            c_arr.iter().filter_map(|item| item.get("text").and_then(|t| t.as_str())).collect::<Vec<_>>().join("\n")
                                        } else if let Some(s) = l_val.get("content").and_then(|c| c.as_str()) {
                                            s.to_string()
                                        } else {
                                            String::new()
                                        };

                                        if !text.is_empty() && role != "system" {
                                            msgs.push(serde_json::json!({
                                                "role": role,
                                                "content": text
                                            }));
                                        }
                                    }
                                }
                            }
                        }

                        if let Some(obj) = sum_val.as_object_mut() {
                            obj.insert("messages".to_string(), serde_json::json!(msgs));
                        }
                        return Ok(sum_val);
                    }
                }
            }
        }
    }

    Err(format!("Session not found with ID or prefix '{}'", id))
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
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WorkspaceEntry {
    pub path: String,
    pub name: String,
    pub is_dir: bool,
}

#[tauri::command]
fn list_workspace_entries(workspace_dir: Option<String>) -> Result<Vec<WorkspaceEntry>, String> {
    let root = match workspace_dir {
        Some(dir) if !dir.trim().is_empty() => PathBuf::from(dir),
        _ => std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    };

    if !root.exists() {
        return Ok(Vec::new());
    }

    let mut entries = Vec::new();
    let mut stack = vec![root.clone()];
    let max_entries = 500;

    let ignored_names = [
        "target", "node_modules", ".git", "dist", "build", ".DS_Store",
        ".next", ".svelte-kit", ".turbo", "coverage", ".fusion",
    ];

    while let Some(current_dir) = stack.pop() {
        if entries.len() >= max_entries {
            break;
        }

        let read_dir = match fs::read_dir(&current_dir) {
            Ok(rd) => rd,
            Err(_) => continue,
        };

        for entry_res in read_dir.flatten() {
            let path = entry_res.path();
            let name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n,
                None => continue,
            };

            if name.starts_with('.') && name != ".env" && name != ".github" && name != ".claude" && name != ".agents" {
                continue;
            }

            if ignored_names.contains(&name) {
                continue;
            }

            let is_dir = path.is_dir();
            let rel_path = path
                .strip_prefix(&root)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| name.to_string());

            entries.push(WorkspaceEntry {
                path: rel_path,
                name: name.to_string(),
                is_dir,
            });

            if is_dir && entries.len() < max_entries {
                stack.push(path);
            }

            if entries.len() >= max_entries {
                break;
            }
        }
    }

    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.path.cmp(&b.path)));

    Ok(entries)
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TerminalExecutionResult {
    pub command: String,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub success: bool,
}

#[tauri::command]
async fn execute_terminal_command(
    command: String,
    cwd: Option<String>,
) -> Result<TerminalExecutionResult, String> {
    let raw_cmd = command.trim();
    if raw_cmd.is_empty() {
        return Err("command cannot be empty".to_string());
    }

    let work_dir = match cwd {
        Some(dir) if !dir.trim().is_empty() => PathBuf::from(dir),
        _ => std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    };

    let mut process = std::process::Command::new("/bin/sh");
    process.arg("-c").arg(raw_cmd);
    process.current_dir(work_dir);

    let path_env = std::env::var("PATH").unwrap_or_default();
    let enriched_path = format!(
        "/Users/aungmyatmoe/.cargo/bin:/opt/homebrew/opt/rustup/bin:/opt/homebrew/bin:{}",
        path_env
    );
    process.env("PATH", enriched_path);

    let output = process
        .output()
        .map_err(|e| format!("Failed to execute command: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let exit_code = output.status.code().unwrap_or(if output.status.success() { 0 } else { 1 });

    Ok(TerminalExecutionResult {
        command: raw_cmd.to_string(),
        stdout,
        stderr,
        exit_code,
        success: output.status.success(),
    })
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GitFileChange {
    pub path: String,
    pub status: String,
    pub additions: usize,
    pub deletions: usize,
    pub patch: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WorkspaceGitStatus {
    pub branch: String,
    pub changes: Vec<GitFileChange>,
    pub full_diff: String,
}

#[tauri::command]
fn get_workspace_git_diff(cwd: Option<String>) -> Result<WorkspaceGitStatus, String> {
    let work_dir = match cwd {
        Some(dir) if !dir.trim().is_empty() => PathBuf::from(dir),
        _ => std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    };

    // 1. Get current branch
    let branch_output = std::process::Command::new("git")
        .arg("rev-parse")
        .arg("--abbrev-ref")
        .arg("HEAD")
        .current_dir(&work_dir)
        .output()
        .ok();
    let branch = branch_output
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "main".to_string());

    // 2. Get full uncommitted git diff
    let full_diff_output = std::process::Command::new("git")
        .arg("diff")
        .arg("HEAD")
        .current_dir(&work_dir)
        .output()
        .ok();
    let full_diff = full_diff_output
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();

    // 3. Get status porcelain
    let status_output = std::process::Command::new("git")
        .arg("status")
        .arg("--porcelain=v1")
        .current_dir(&work_dir)
        .output()
        .ok();

    let mut changes = Vec::new();
    if let Some(out) = status_output {
        let stdout_str = String::from_utf8_lossy(&out.stdout);
        for line in stdout_str.lines() {
            let line = line.trim_end();
            if line.len() < 3 {
                continue;
            }
            let status_code = line[..2].trim().to_string();
            let file_path = line[3..].trim().to_string();

            // Run per-file diff
            let file_diff_out = std::process::Command::new("git")
                .arg("diff")
                .arg("HEAD")
                .arg("--")
                .arg(&file_path)
                .current_dir(&work_dir)
                .output()
                .ok();

            let patch = file_diff_out
                .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                .unwrap_or_default();

            let mut additions = 0;
            let mut deletions = 0;
            for pl in patch.lines() {
                if pl.starts_with('+') && !pl.starts_with("+++") {
                    additions += 1;
                } else if pl.starts_with('-') && !pl.starts_with("---") {
                    deletions += 1;
                }
            }

            changes.push(GitFileChange {
                path: file_path,
                status: if status_code.is_empty() { "M".to_string() } else { status_code },
                additions,
                deletions,
                patch,
            });
        }
    }

    Ok(WorkspaceGitStatus {
        branch,
        changes,
        full_diff,
    })
}
#[tauri::command]
fn show_desktop_notification(
    app: tauri::AppHandle,
    title: String,
    body: String,
    session_id: Option<String>,
    sound: Option<bool>,
) -> Result<(), String> {
    let title = title.trim();
    let body = body.trim();
    if title.is_empty() || body.is_empty() {
        return Err("notification title and body are required".to_string());
    }

    let should_sound = sound.unwrap_or(true);

    let mut notification = notify_rust::Notification::new();
    notification
        .summary(title)
        .body(body)
        .auto_icon()
        .action("open-session", "Open");

    if should_sound {
        #[cfg(target_os = "macos")]
        notification.sound_name("default");
        #[cfg(not(target_os = "macos"))]
        notification.sound_name("Default");
    }

    #[cfg(target_os = "windows")]
    {
        notification.app_id(&app.config().identifier);
    }

    #[cfg(target_os = "macos")]
    macos_notification::configure(&app)?;

    let handle = notification
        .show()
        .map_err(|error| format!("failed showing notification: {error}"))?;

    if let Some(sid) = session_id {
        let clean_sid = sid.trim().to_string();
        if !clean_sid.is_empty() {
            let app_handle = app.clone();
            std::thread::spawn(move || {
                if let Err(error) = handle.wait_for_response(move |response: &notify_rust::NotificationResponse| {
                    if notification_response_opens_session(response) {
                        if let Some(window) = app_handle.get_webview_window("main") {
                            let _ = window.unminimize();
                            let _ = window.set_focus();
                        }
                        let _ = app_handle.emit("open_session", serde_json::json!({ "sessionId": clean_sid }));
                    }
                }) {
                    eprintln!("[notification] wait_for_response error: {error}");
                }
            });
        }
    }

    Ok(())
}

#[tauri::command]
fn play_system_sound(sound: Option<String>) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let name = sound.unwrap_or_else(|| "Ping".to_string());
        let path = format!("/System/Library/Sounds/{}.aiff", name);
        if std::path::Path::new(&path).exists() {
            let _ = std::process::Command::new("afplay")
                .arg(&path)
                .spawn();
        } else {
            let _ = std::process::Command::new("afplay")
                .arg("/System/Library/Sounds/Ping.aiff")
                .spawn();
        }
    }
    Ok(())
}
fn fusion_dir() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".fusion");
    }
    if let Ok(userprofile) = std::env::var("USERPROFILE") {
        return PathBuf::from(userprofile).join(".fusion");
    }
    PathBuf::from(".fusion")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthStatus {
    pub is_signed_in: bool,
    pub name: Option<String>,
    pub email: Option<String>,
    pub provider: Option<String>,
}

#[tauri::command]
fn check_auth_status() -> AuthStatus {
    let dir = fusion_dir();

    // 1. Check ~/.fusion/config.json
    let config_path = dir.join("config.json");
    if config_path.exists() {
        if let Ok(content) = fs::read_to_string(&config_path) {
            if let Ok(val) = serde_json::from_str::<Value>(&content) {
                if let Some(key) = val.get("fusion_api_key").and_then(|k| k.as_str()) {
                    if !key.trim().is_empty() {
                        return AuthStatus {
                            is_signed_in: true,
                            name: val.get("name").and_then(|n| n.as_str()).map(|s| s.to_string()),
                            email: val.get("email").and_then(|e| e.as_str()).map(|s| s.to_string()),
                            provider: Some("fusion".to_string()),
                        };
                    }
                }
                if let Some(key) = val.get("anthropic_api_key").and_then(|k| k.as_str()) {
                    if !key.trim().is_empty() {
                        return AuthStatus {
                            is_signed_in: true,
                            name: None,
                            email: None,
                            provider: Some("anthropic".to_string()),
                        };
                    }
                }
            }
        }
    }
    // 2. Check ~/.fusion/auth.json
    let auth_path = dir.join("auth.json");
    if auth_path.exists() {
        if let Ok(content) = fs::read_to_string(&auth_path) {
            if let Ok(val) = serde_json::from_str::<Value>(&content) {
                if let Some(obj) = val.as_object() {
                    if !obj.is_empty() {
                        return AuthStatus {
                            is_signed_in: true,
                            name: None,
                            email: None,
                            provider: Some("fusion".to_string()),
                        };
                    }
                }
            }
        }
    }

    // 3. Check environment variables
    for env_key in &["FUSION_API_KEY", "ANTHROPIC_API_KEY", "OPENAI_API_KEY", "XAI_API_KEY", "DEEPSEEK_API_KEY"] {
        if let Ok(val) = std::env::var(env_key) {
            if !val.trim().is_empty() {
                return AuthStatus {
                    is_signed_in: true,
                    name: None,
                    email: None,
                    provider: Some(env_key.to_lowercase()),
                };
            }
        }
    }

    AuthStatus {
        is_signed_in: false,
        name: None,
        email: None,
        provider: None,
    }
}

#[tauri::command]
async fn start_fusion_login() -> Result<AuthStatus, String> {
    let bin = find_fusion_binary().unwrap_or_else(|_| PathBuf::from("fusion"));
    let mut child = tokio::process::Command::new(bin)
        .arg("login")
        .spawn()
        .map_err(|e| format!("Failed to run fusion login: {}", e))?;

    let _ = child.wait().await;
    Ok(check_auth_status())
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
    if output.status.success() || !cleaned_stdout.is_empty() {
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

    while let Ok(Ok(Some(line))) = tokio::time::timeout(tokio::time::Duration::from_secs(45), reader.next_line()).await {
        if line.contains("\"id\":3") || line.contains("\"id\": 3") {
            break;
        }

        let line_to_parse = if let Some(idx) = line.find('{') {
            &line[idx..]
        } else {
            continue;
        };

        if let Ok(val) = serde_json::from_str::<Value>(line_to_parse) {
            if val.get("id") == Some(&serde_json::json!(3))
                || val.get("result").and_then(|r| r.get("stopReason")).is_some()
            {
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
            execute_terminal_command,
            get_workspace_git_diff,
            list_workspace_entries,
            pick_project_folder,
            execute_fusion_turn,
            stream_fusion_acp,
            check_auth_status,
            start_fusion_login,
            show_desktop_notification,
            play_system_sound,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
