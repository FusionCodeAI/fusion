use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::tools::file::resolve_path;
use crate::tools::types::{Tool, ToolContext};

/// Structured representation of command execution output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub success: bool,
}

impl CommandOutput {
    /// Returns the combined stdout and stderr representation.
    pub fn combined(&self) -> String {
        let mut out = String::new();
        if !self.stdout.is_empty() {
            out.push_str(&self.stdout);
        }
        if !self.stderr.is_empty() {
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(&self.stderr);
        }
        out
    }
}

/// Cross-platform shell tool for executing bash/sh/cmd commands with timeout enforcement,
/// kill-on-drop process handling, working directory resolution, and stdout/stderr capture.
#[derive(Default, Debug, Clone)]
pub struct BashTool;

impl BashTool {
    pub fn new() -> Self {
        Self
    }

    /// Construct a platform-specific shell Command.
    ///
    /// - Windows: Uses %COMSPEC% or cmd.exe with `/C`
    /// - Android/Termux: Checks $SHELL, Termux sh path, or standard sh with `-c`
    /// - Linux/macOS: Uses $SHELL or /bin/sh with `-c`
    pub fn build_command(command: &str) -> tokio::process::Command {
        #[cfg(windows)]
        {
            let shell = std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string());
            let mut cmd = tokio::process::Command::new(shell);
            cmd.arg("/C").arg(command);
            cmd
        }

        #[cfg(not(windows))]
        {
            let shell = std::env::var("SHELL")
                .ok()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| {
                    if cfg!(target_os = "android") {
                        if Path::new("/data/data/com.termux/files/usr/bin/sh").exists() {
                            "/data/data/com.termux/files/usr/bin/sh".to_string()
                        } else if Path::new("/system/bin/sh").exists() {
                            "/system/bin/sh".to_string()
                        } else {
                            "sh".to_string()
                        }
                    } else if Path::new("/bin/sh").exists() {
                        "/bin/sh".to_string()
                    } else {
                        "sh".to_string()
                    }
                });

            let mut cmd = tokio::process::Command::new(shell);
            cmd.arg("-c").arg(command);
            cmd
        }
    }

    /// Execute a shell command directly with custom parameters.
    pub async fn run_in_context(
        &self,
        command: &str,
        working_dir: &Path,
        timeout_secs: u64,
        env: Option<&HashMap<String, String>>,
    ) -> anyhow::Result<CommandOutput> {
        let trimmed_cmd = command.trim();
        if trimmed_cmd.is_empty() {
            anyhow::bail!("Command cannot be empty");
        }

        if !working_dir.exists() {
            anyhow::bail!(
                "Working directory does not exist: {}",
                working_dir.display()
            );
        }

        if !working_dir.is_dir() {
            anyhow::bail!(
                "Working directory path is not a directory: {}",
                working_dir.display()
            );
        }

        let mut cmd = Self::build_command(trimmed_cmd);

        cmd.current_dir(working_dir);
        // Sanitize environment to strip API keys, secrets, and credentials before spawning subprocess
        let cleaner = crate::tools::env_cleaner::EnvCleaner::default();
        cleaner.apply_to_tokio_command(&mut cmd, env);

        cmd.stdin(std::process::Stdio::null());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        cmd.kill_on_drop(true);

        let child = cmd
            .spawn()
            .map_err(|e| anyhow::anyhow!("Failed to spawn shell command '{}': {e}", trimmed_cmd))?;

        let timeout_secs_clamped = timeout_secs.max(1);
        let timeout_duration = Duration::from_secs(timeout_secs_clamped);

        let output = match tokio::time::timeout(timeout_duration, child.wait_with_output()).await {
            Ok(Ok(out)) => out,
            Ok(Err(e)) => {
                anyhow::bail!("Failed to read output from command '{}': {e}", trimmed_cmd);
            }
            Err(_) => {
                anyhow::bail!(
                    "Command timed out after {} second{}: {}",
                    timeout_secs_clamped,
                    if timeout_secs_clamped == 1 { "" } else { "s" },
                    trimmed_cmd
                );
            }
        };

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let exit_code = output.status.code();
        let success = output.status.success();

        Ok(CommandOutput {
            stdout,
            stderr,
            exit_code,
            success,
        })
    }
}

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "Execute a shell command (Unix/Android: sh -c, Windows: cmd.exe /C) in the workspace directory with timeout enforcement and process isolation."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The shell command to execute."
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Execution timeout in seconds (optional, default: 60)."
                },
                "cwd": {
                    "type": "string",
                    "description": "Optional working directory path relative to workspace or absolute."
                },
                "env": {
                    "type": "object",
                    "description": "Optional extra environment variables key-value mapping.",
                    "additionalProperties": {
                        "type": "string"
                    }
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> anyhow::Result<String> {
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("cmd").and_then(|v| v.as_str()))
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: command"))?;

        if command.trim().is_empty() {
            anyhow::bail!("Command cannot be empty");
        }

        let timeout_secs = args
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .or_else(|| args.get("timeout").and_then(|v| v.as_u64()))
            .unwrap_or(60);

        // Resolve target working directory
        let working_dir = if let Some(cwd_str) = args
            .get("cwd")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("working_directory").and_then(|v| v.as_str()))
            .or_else(|| args.get("workdir").and_then(|v| v.as_str()))
            .or_else(|| args.get("dir").and_then(|v| v.as_str()))
        {
            if cwd_str.trim().is_empty() {
                ctx.cwd.clone()
            } else {
                resolve_path(cwd_str, &ctx.cwd)
            }
        } else {
            ctx.cwd.clone()
        };

        // Prepare environment variables (context env merged with optional per-command env)
        let mut merged_env = ctx.env.clone();
        if let Some(extra_env) = args.get("env").and_then(|v| v.as_object()) {
            for (k, v) in extra_env {
                if let Some(val_str) = v.as_str() {
                    merged_env.insert(k.clone(), val_str.to_string());
                }
            }
        }

        let output = self
            .run_in_context(command, &working_dir, timeout_secs, Some(&merged_env))
            .await?;

        let combined = output.combined();

        if !output.success {
            #[cfg(unix)]
            let exit_desc = {
                if let Some(code) = output.exit_code {
                    format!("exit code {code}")
                } else {
                    "terminated by signal".to_string()
                }
            };

            #[cfg(not(unix))]
            let exit_desc = {
                if let Some(code) = output.exit_code {
                    format!("exit code {code}")
                } else {
                    "abnormal termination".to_string()
                }
            };

            let trimmed = combined.trim();
            if trimmed.is_empty() {
                anyhow::bail!("Command failed with {}", exit_desc);
            } else {
                anyhow::bail!("Command failed with {}:\n{}", exit_desc, trimmed);
            }
        }

        if combined.trim().is_empty() {
            Ok("(command completed with no output)".to_string())
        } else {
            Ok(combined)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::Instant;

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new() -> Self {
            let unique_id = uuid::Uuid::new_v4();
            let path = std::env::temp_dir().join(format!("fusion_bash_test_{unique_id}"));
            fs::create_dir_all(&path).expect("failed to create temp dir");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[tokio::test]
    async fn test_bash_simple_echo() {
        let temp = TestDir::new();
        let ctx = ToolContext {
            cwd: temp.path().to_path_buf(),
            env: HashMap::new(),
        };

        let tool = BashTool::new();
        let res = tool
            .execute(json!({ "command": "echo 'Fusion v2 Shell Test'" }), &ctx)
            .await
            .unwrap();

        assert!(res.contains("Fusion v2 Shell Test"));
    }

    #[tokio::test]
    async fn test_bash_exit_code_failure() {
        let temp = TestDir::new();
        let ctx = ToolContext {
            cwd: temp.path().to_path_buf(),
            env: HashMap::new(),
        };

        let tool = BashTool::new();
        #[cfg(not(windows))]
        let cmd_str = "echo 'failing soon' >&2; exit 42";
        #[cfg(windows)]
        let cmd_str = "echo failing soon 1>&2 & exit /b 42";

        let err = tool
            .execute(json!({ "command": cmd_str }), &ctx)
            .await
            .unwrap_err();

        let err_msg = err.to_string();
        assert!(err_msg.contains("42"));
        assert!(err_msg.contains("failing soon"));
    }

    #[tokio::test]
    async fn test_bash_timeout_enforcement() {
        let temp = TestDir::new();
        let ctx = ToolContext {
            cwd: temp.path().to_path_buf(),
            env: HashMap::new(),
        };

        let tool = BashTool::new();
        #[cfg(not(windows))]
        let cmd_str = "sleep 10";
        #[cfg(windows)]
        let cmd_str = "ping -n 11 127.0.0.1 >nul";

        let start = Instant::now();
        let err = tool
            .execute(
                json!({
                    "command": cmd_str,
                    "timeout_secs": 1
                }),
                &ctx,
            )
            .await
            .unwrap_err();

        let elapsed = start.elapsed();
        assert!(
            elapsed.as_secs() < 5,
            "Timeout took too long: {:?}",
            elapsed
        );
        assert!(err.to_string().contains("timed out"));
    }

    #[tokio::test]
    async fn test_bash_working_directory_resolution() {
        let temp = TestDir::new();
        let sub_dir = temp.path().join("subdir");
        fs::create_dir_all(&sub_dir).unwrap();

        let ctx = ToolContext {
            cwd: temp.path().to_path_buf(),
            env: HashMap::new(),
        };

        let tool = BashTool::new();
        #[cfg(not(windows))]
        let cmd_str = "pwd";
        #[cfg(windows)]
        let cmd_str = "cd";

        let res = tool
            .execute(
                json!({
                    "command": cmd_str,
                    "cwd": "subdir"
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(res.contains("subdir"));
    }

    #[tokio::test]
    async fn test_bash_invalid_working_directory() {
        let temp = TestDir::new();
        let ctx = ToolContext {
            cwd: temp.path().to_path_buf(),
            env: HashMap::new(),
        };

        let tool = BashTool::new();
        let err = tool
            .execute(
                json!({
                    "command": "echo hello",
                    "cwd": "non_existent_directory_xyz"
                }),
                &ctx,
            )
            .await
            .unwrap_err();

        assert!(err.to_string().contains("Working directory does not exist"));
    }

    #[tokio::test]
    async fn test_bash_custom_env() {
        let temp = TestDir::new();
        let mut base_env = HashMap::new();
        base_env.insert("BASE_VAR".to_string(), "BaseValue".to_string());

        let ctx = ToolContext {
            cwd: temp.path().to_path_buf(),
            env: base_env,
        };

        let tool = BashTool::new();
        #[cfg(not(windows))]
        let cmd_str = "echo $BASE_VAR-$CUSTOM_VAR";
        #[cfg(windows)]
        let cmd_str = "echo %BASE_VAR%-%CUSTOM_VAR%";

        let res = tool
            .execute(
                json!({
                    "command": cmd_str,
                    "env": {
                        "CUSTOM_VAR": "CustomValue"
                    }
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(res.contains("BaseValue-CustomValue"));
    }

    #[tokio::test]
    async fn test_bash_empty_command() {
        let temp = TestDir::new();
        let ctx = ToolContext {
            cwd: temp.path().to_path_buf(),
            env: HashMap::new(),
        };

        let tool = BashTool::new();
        let err = tool
            .execute(json!({ "command": "   " }), &ctx)
            .await
            .unwrap_err();

        assert!(err.to_string().contains("cannot be empty"));
    }

    #[tokio::test]
    async fn test_bash_aliases() {
        let temp = TestDir::new();
        let ctx = ToolContext {
            cwd: temp.path().to_path_buf(),
            env: HashMap::new(),
        };

        let tool = BashTool::new();
        let res = tool
            .execute(
                json!({
                    "cmd": "echo 'Alias test'",
                    "timeout": 30
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(res.contains("Alias test"));
    }

    #[tokio::test]
    async fn test_bash_run_in_context_direct() {
        let temp = TestDir::new();
        let tool = BashTool::new();

        let out = tool
            .run_in_context("echo direct", temp.path(), 10, None)
            .await
            .unwrap();

        assert!(out.success);
        assert_eq!(out.exit_code, Some(0));
        assert!(out.stdout.contains("direct"));
    }

    #[tokio::test]
    async fn test_bash_strips_api_keys_and_secrets() {
        let temp = TestDir::new();
        let mut base_env = HashMap::new();
        base_env.insert(
            "OPENAI_API_KEY".to_string(),
            "sk-proj-super-secret-key-12345".to_string(),
        );
        base_env.insert("SAFE_APP_ENV".to_string(), "production-v2".to_string());

        let ctx = ToolContext {
            cwd: temp.path().to_path_buf(),
            env: base_env,
        };

        let tool = BashTool::new();
        #[cfg(not(windows))]
        let cmd_str = "echo OPENAI=$OPENAI_API_KEY ANTHROPIC=$ANTHROPIC_API_KEY SAFE=$SAFE_APP_ENV";
        #[cfg(windows)]
        let cmd_str =
            "echo OPENAI=%OPENAI_API_KEY% ANTHROPIC=%ANTHROPIC_API_KEY% SAFE=%SAFE_APP_ENV%";

        let res = tool
            .execute(
                json!({
                    "command": cmd_str,
                    "env": {
                        "ANTHROPIC_API_KEY": "sk-ant-super-secret-key-67890"
                    }
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(
            !res.contains("sk-proj-super-secret-key-12345"),
            "OpenAI API key leaked in command output: {}",
            res
        );
        assert!(
            !res.contains("sk-ant-super-secret-key-67890"),
            "Anthropic API key leaked in command output: {}",
            res
        );
        assert!(
            res.contains("production-v2"),
            "Safe variable missing from command output: {}",
            res
        );
    }
}
