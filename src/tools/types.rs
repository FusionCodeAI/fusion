use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::provider::types::ToolDefinition;

#[derive(Debug, Clone)]
pub struct ToolContext {
    pub cwd: PathBuf,
    pub env: HashMap<String, String>,
}

impl Default for ToolContext {
    fn default() -> Self {
        Self {
            cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            env: std::env::vars().collect(),
        }
    }
}

impl ToolContext {
    /// Creates a new ToolContext with the host environment pre-sanitized
    /// to strip API keys, secrets, and credentials.
    pub fn new_sanitized(cwd: PathBuf) -> Self {
        Self {
            cwd,
            env: crate::tools::env_cleaner::sanitize_env(&std::env::vars().collect()),
        }
    }

    /// Returns a sanitized copy of this context's environment variables.
    pub fn sanitized_env(&self) -> HashMap<String, String> {
        crate::tools::env_cleaner::sanitize_env(&self.env)
    }
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters(&self) -> Value;

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: self.description().to_string(),
            parameters: self.parameters(),
        }
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> anyhow::Result<String>;
}

pub type DynTool = Arc<dyn Tool>;

#[derive(Default, Clone)]
pub struct ToolRegistry {
    tools: HashMap<String, DynTool>,
}

impl std::fmt::Debug for ToolRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolRegistry")
            .field("tools", &self.tools.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    pub fn register(&mut self, tool: DynTool) {
        self.tools.insert(tool.name().to_string(), tool);
    }
    /// Unregister and remove a tool by name.
    pub fn unregister(&mut self, name: &str) -> Option<DynTool> {
        self.tools.remove(name)
    }

    /// Register multiple tools at once.
    pub fn register_all(&mut self, tools: impl IntoIterator<Item = DynTool>) {
        for tool in tools {
            self.register(tool);
        }
    }

    /// Returns a list of all registered tool names.
    pub fn names(&self) -> Vec<String> {
        self.tools.keys().cloned().collect()
    }

    /// Checks if a tool with the given name is registered (including aliases).
    pub fn contains(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// Returns the number of registered tools.
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    /// Returns true if no tools are registered.
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// Clears all registered tools.
    pub fn clear(&mut self) {
        self.tools.clear();
    }


    pub fn get(&self, name: &str) -> Option<DynTool> {
        self.tools.get(name).cloned().or_else(|| match name {
            "read_file" | "view" | "cat" | "readFile" | "display_file" | "show_file" => {
                self.tools.get("read").cloned()
            }
            "write_file" | "create" | "writeFile" | "create_file" | "new_file" => {
                self.tools.get("write").cloned()
            }
            "edit_file" | "str_replace_editor" | "str_replace" | "strreplace" | "editFile"
            | "replace" => self.tools.get("edit").cloned(),
            "terminal" | "shell" | "sh" | "cmd" | "exec" | "execute" | "run_command"
            | "runCommand" => self.tools.get("bash").cloned(),
            "read" => self.tools.get("read_file").cloned(),
            "write" => self.tools.get("write_file").cloned(),
            "edit" => self.tools.get("edit_file").cloned(),
            "status" | "git-status" => self.tools.get("git_status").cloned(),
            "diff" | "git-diff" => self.tools.get("git_diff").cloned(),
            "apply_patch" => self.tools.get("patch").cloned(),
            "file_watch" | "watch_file" | "file_watcher" => self.tools.get("watch").cloned(),
            "websearch" | "search_web" => self.tools.get("web_search").cloned(),
            "clip" | "pbcopy" | "pbpaste" | "xclip" | "xsel" | "wl-copy" | "wl-paste"
            | "clipboard_read" | "clipboard_write" | "read_clipboard" | "write_clipboard"
            | "copy" | "paste" => self.tools.get("clipboard").cloned(),
            "system" | "sys_info" | "sysinfo" | "host_info" | "systeminfo" => {
                self.tools.get("system_info").cloned()
            }
            "syntax" | "syntax_check" | "check_syntax" | "validate_syntax" => {
                self.tools.get("syntax_check").cloned()
            }
            "bg_process" | "background_process" | "proc" | "process_manager" | "processes" => {
                self.tools.get("process").cloned()
            }
            "http_fetch" | "web_fetch" | "http_get" | "curl" | "fetch_web" | "web_page" => {
                self.tools.get("fetch").cloned()
            }
            "symbol" | "workspace_symbols" | "lookup_symbol" | "find_symbols" | "symbol_search" => {
                self.tools.get("symbols").cloned()
            }
            "regex" | "test_regex" | "regex_tester" | "re_test" | "regexp" | "regex_eval" => {
                self.tools.get("regex_test").cloned()
            }
            "hex" | "hex_viewer" | "hexdump" | "hex_dump" | "xxd" | "od" | "binary_view" => {
                self.tools.get("hex_view").cloned()
            }
            "github" | "gh" | "gh_pr" | "github_pr" | "gh_issue" | "github_issue" | "pull_request" | "pull_requests" => {
                self.tools.get("github").cloned()
            }
            "schema" | "json_schema" | "schema_validator" | "validate_schema" | "json_validator" | "validate_json" => {
                self.tools.get("json_schema").cloned()
            }
            _ => None,
        })
    }
    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools.values().map(|t| t.definition()).collect()
    }

    pub async fn execute(&self, name: &str, args: Value, ctx: &ToolContext) -> anyhow::Result<String> {
        let tool = self.get(name).ok_or_else(|| anyhow::anyhow!("Unknown tool: {}", name))?;
        tool.execute(args, ctx).await
    }
}
