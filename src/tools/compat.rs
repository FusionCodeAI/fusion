use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use crate::provider::types::ToolDefinition;
use crate::tools::bash::BashTool;
use crate::tools::edit::EditFileTool;
use crate::tools::file::{resolve_path, ReadFileTool, WriteFileTool};
use crate::tools::types::{DynTool, Tool, ToolContext, ToolRegistry};

// ===========================================================================
// Canonical Mapping Table & Normalization
// ===========================================================================

/// Known tool aliases mapped to canonical Fusion tool names.
pub const KNOWN_TOOL_ALIASES: &[(&str, &str)] = &[
    // Edit / Code Modification aliases
    ("str_replace_editor", "edit"),
    ("str_replace", "edit"),
    ("strreplace", "edit"),
    ("edit_file", "edit"),
    ("editFile", "edit"),
    ("replace", "edit"),
    ("replace_in_file", "edit"),
    // Read / View / Cat aliases
    ("view", "read"),
    ("cat", "read"),
    ("read_file", "read"),
    ("readFile", "read"),
    ("display_file", "read"),
    ("show_file", "read"),
    ("read_file_tool", "read"),
    // Write / Create aliases
    ("create", "write"),
    ("create_file", "write"),
    ("createFile", "write"),
    ("write_file", "write"),
    ("writeFile", "write"),
    ("new_file", "write"),
    ("overwrite_file", "write"),
    ("put", "write"),
    // Bash / Terminal aliases
    ("terminal", "bash"),
    ("shell", "bash"),
    ("sh", "bash"),
    ("cmd", "bash"),
    ("exec", "bash"),
    ("execute", "bash"),
    ("run_command", "bash"),
    ("runCommand", "bash"),
    ("bash_command", "bash"),
    ("command_runner", "bash"),
    // Search / Grep aliases
    ("search", "grep"),
    ("search_files", "grep"),
    ("find_in_files", "grep"),
    ("ripgrep", "grep"),
    ("rg", "grep"),
    // Glob / File Find aliases
    ("find_files", "glob"),
    ("list_files", "glob"),
    ("locate_files", "glob"),
    // Regex Tester aliases
    ("regex", "regex_test"),
    ("test_regex", "regex_test"),
    ("regex_tester", "regex_test"),
    ("re_test", "regex_test"),
    ("regexp", "regex_test"),
    ("regex_eval", "regex_test"),
    // Mock Server aliases
    ("mock_http", "mock_server"),
    ("mock_http_server", "mock_server"),
    ("http_mock", "mock_server"),
    ("mockserver", "mock_server"),
    ("http_mock_server", "mock_server"),
    // Tree aliases
    ("dir_tree", "tree"),
    ("dirtree", "tree"),
    ("directory_tree", "tree"),
    ("file_tree", "tree"),
    // Dependency Auditor aliases
    ("dependencies", "deps"),
    ("dependency_auditor", "deps"),
    ("audit_deps", "deps"),
    ("check_deps", "deps"),
    ("deps_audit", "deps"),
    ("outdated_deps", "deps"),
    // Diff Stats aliases
    ("diff_aggregator", "diff_stats"),
    ("session_diff_stats", "diff_stats"),
    ("diff_stat", "diff_stats"),
    ("diffstat", "diff_stats"),
    ("session_diff", "diff_stats"),
    // Git Log aliases
    ("gitlog", "git_log"),
    ("log", "git_log"),
    ("commits", "git_log"),
    ("git_commits", "git_log"),
    ("git_history", "git_log"),
    ("history", "git_log"),
    ("commit_history", "git_log"),
    ("git_log_tool", "git_log"),
    // GitHub aliases
    ("gh", "github"),
    ("gh_pr", "github"),
    ("github_pr", "github"),
    ("gh_issue", "github"),
    ("github_issue", "github"),
    ("pull_request", "github"),
    ("pull_requests", "github"),
    ("github_prs", "github"),
    ("github_issues", "github"),
    ("gh_cli", "github"),
    // JSON Schema aliases
    ("schema", "json_schema"),
    ("schema_validator", "json_schema"),
    ("validate_schema", "json_schema"),
    ("json_validator", "json_schema"),
    ("validate_json", "json_schema"),
    ("schema_check", "json_schema"),
    ("check_schema", "json_schema"),
];

/// Returns the canonical tool name for a given alias or raw tool name.
pub fn canonical_tool_name(name: &str) -> &str {
    let lower = name.trim().to_lowercase();
    for &(alias, canonical) in KNOWN_TOOL_ALIASES {
        if alias.eq_ignore_ascii_case(&lower) {
            return canonical;
        }
    }
    name
}

/// Checks if a given tool name is a known compatibility alias.
pub fn is_compat_alias(name: &str) -> bool {
    let lower = name.trim().to_lowercase();
    KNOWN_TOOL_ALIASES
        .iter()
        .any(|&(alias, _)| alias.eq_ignore_ascii_case(&lower))
}

/// Returns the list of all known alias-to-canonical pairs.
pub fn get_known_aliases() -> &'static [(&'static str, &'static str)] {
    KNOWN_TOOL_ALIASES
}

// ===========================================================================
// Argument Normalization Helpers
// ===========================================================================

/// Extract a string field from JSON value checking multiple alternative keys.
pub fn extract_string_field<'a>(args: &'a Value, keys: &[&str]) -> Option<&'a str> {
    if let Value::Object(map) = args {
        for key in keys {
            if let Some(val) = map.get(*key) {
                if let Some(s) = val.as_str() {
                    return Some(s);
                }
            }
        }
    }
    None
}

/// Extract an unsigned integer field checking multiple alternative keys.
pub fn extract_u64_field(args: &Value, keys: &[&str]) -> Option<u64> {
    if let Value::Object(map) = args {
        for key in keys {
            if let Some(val) = map.get(*key) {
                if let Some(n) = val.as_u64() {
                    return Some(n);
                }
                if let Some(s) = val.as_str() {
                    if let Ok(n) = s.parse::<u64>() {
                        return Some(n);
                    }
                }
            }
        }
    }
    None
}

/// Extract a boolean field checking multiple alternative keys.
pub fn extract_bool_field(args: &Value, keys: &[&str]) -> Option<bool> {
    if let Value::Object(map) = args {
        for key in keys {
            if let Some(val) = map.get(*key) {
                if let Some(b) = val.as_bool() {
                    return Some(b);
                }
                if let Some(s) = val.as_str() {
                    if s.eq_ignore_ascii_case("true") {
                        return Some(true);
                    } else if s.eq_ignore_ascii_case("false") {
                        return Some(false);
                    }
                }
            }
        }
    }
    None
}

/// Normalize a path argument for cross-platform handling:
/// - strips surrounding double quotes
/// - converts Windows-style `\` separators to `/`
/// - preserves verbatim (`\\?\`), device (`\\.\`), and UNC (`\\server\share`)
///   prefixes untouched, and leaves `cwd`-style working directories alone
pub fn normalize_path_arg(path: &str) -> String {
    let s = path.trim();
    let s = if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        &s[1..s.len() - 1]
    } else {
        s
    };
    if s.starts_with("\\\\?\\") || s.starts_with("\\\\.\\") || s.starts_with("\\\\") {
        // Windows verbatim/device/UNC prefixes: keep as-is.
        return s.to_string();
    }
    s.replace('\\', "/")
}

/// Applies `normalize_path_arg` to every string-valued path-like key in `args`.
fn normalize_path_fields(args: &mut Value, keys: &[&str]) {
    if let Value::Object(map) = args {
        for key in keys {
            if let Some(val) = map.get_mut(*key) {
                if let Some(s) = val.as_str() {
                    *val = Value::String(normalize_path_arg(s));
                }
            }
        }
    }
}

/// Extracts a path field checking multiple alternative keys, normalizing the
/// result through `normalize_path_arg`.
pub fn extract_path_field<'a>(args: &'a Value, keys: &[&str]) -> Option<String> {
    extract_string_field(args, keys).map(normalize_path_arg)
}
/// Normalizes tool arguments from external formats into Fusion's canonical format.
pub fn normalize_tool_args(canonical_name: &str, args: &Value) -> Value {
    match canonical_name {
        "edit" => {
            let path = extract_path_field(
                args,
                &["path", "file_path", "filePath", "file", "filename", "target_file"],
            )
            .unwrap_or_default();

            let old_text = extract_string_field(
                args,
                &[
                    "old_text",
                    "old_str",
                    "oldText",
                    "search",
                    "find",
                    "original_text",
                    "old",
                ],
            )
            .unwrap_or_default();

            let new_text = extract_string_field(
                args,
                &[
                    "new_text",
                    "new_str",
                    "newText",
                    "replace",
                    "replacement",
                    "new",
                ],
            )
            .unwrap_or_default();

            json!({
                "path": path,
                "old_text": old_text,
                "new_text": new_text,
            })
        }
        "read" => {
            let path = extract_path_field(
                args,
                &["path", "file_path", "filePath", "file", "filename", "path_str", "target"],
            )
            .unwrap_or_default();

            let mut offset = extract_u64_field(
                args,
                &["offset", "start_line", "start", "line_start", "line_offset", "from_line", "begin"],
            );

            let mut limit = extract_u64_field(
                args,
                &["limit", "num_lines", "count", "length", "lines", "to_line"],
            );

            // Handle view_range: [start, end]
            if let Some(range) = args.get("view_range").and_then(|v| v.as_array()) {
                if !range.is_empty() {
                    if let Some(start) = range.first().and_then(|v| v.as_u64()) {
                        offset = Some(start);
                        if let Some(end) = range.get(1).and_then(|v| v.as_u64()) {
                            if end >= start {
                                limit = Some(end - start + 1);
                            }
                        }
                    }
                }
            }

            let line_numbers = extract_bool_field(
                args,
                &["line_numbers", "line_number", "numbers", "show_line_numbers"],
            );

            let mut res = json!({ "path": path });
            if let Some(off) = offset {
                res["offset"] = json!(off);
            }
            if let Some(lim) = limit {
                res["limit"] = json!(lim);
            }
            if let Some(ln) = line_numbers {
                res["line_numbers"] = json!(ln);
            }
            res
        }
        "write" => {
            let path = extract_path_field(
                args,
                &["path", "file_path", "filePath", "file", "filename", "path_str", "dest", "destination", "target"],
            )
            .unwrap_or_default();

            let content = extract_string_field(
                args,
                &["content", "file_text", "text", "contents", "file_content", "data", "body", "code"],
            )
            .unwrap_or_default();

            json!({
                "path": path,
                "content": content,
            })
        }
        "bash" => {
            let command = extract_string_field(
                args,
                &["command", "cmd", "input", "script", "run", "code", "line", "exec", "cli"],
            )
            .unwrap_or_default();

            let timeout = extract_u64_field(
                args,
                &["timeout", "timeout_secs", "timeout_seconds"],
            );

            let cwd = extract_path_field(
                args,
                &["cwd", "workdir", "working_directory", "dir"],
            );

            let mut res = json!({ "command": command });
            if let Some(t) = timeout {
                res["timeout"] = json!(t);
            }
            if let Some(d) = cwd {
                res["cwd"] = json!(d);
            }
            res
        }
        "git_log" => {
            let path = extract_string_field(
                args,
                &["path", "repo_path", "target_dir", "dir", "cwd"],
            );
            let file_path = extract_string_field(
                args,
                &["file_path", "file", "filename", "target_file", "filepath"],
            );
            let max_count = extract_u64_field(
                args,
                &["max_count", "limit", "count", "n", "num"],
            );
            let skip = extract_u64_field(
                args,
                &["skip", "offset"],
            );
            let revision = extract_string_field(
                args,
                &["revision", "rev", "range", "branch", "ref"],
            );
            let author = extract_string_field(
                args,
                &["author", "user", "committer"],
            );
            let grep = extract_string_field(
                args,
                &["grep", "query", "search", "pattern", "message"],
            );
            let since = extract_string_field(
                args,
                &["since", "after", "from"],
            );
            let until = extract_string_field(
                args,
                &["until", "before", "to"],
            );
            let show_files = extract_bool_field(
                args,
                &["show_files", "files", "stat", "stats", "numstat"],
            );
            let oneline = extract_bool_field(
                args,
                &["oneline", "compact", "short"],
            );
            let format = extract_string_field(
                args,
                &["format", "style", "mode"],
            );

            let mut res = json!({});
            if let Some(p) = path {
                res["path"] = json!(p);
            }
            if let Some(f) = file_path {
                res["file_path"] = json!(f);
            }
            if let Some(m) = max_count {
                res["max_count"] = json!(m);
            }
            if let Some(s) = skip {
                res["skip"] = json!(s);
            }
            if let Some(r) = revision {
                res["revision"] = json!(r);
            }
            if let Some(a) = author {
                res["author"] = json!(a);
            }
            if let Some(g) = grep {
                res["grep"] = json!(g);
            }
            if let Some(sn) = since {
                res["since"] = json!(sn);
            }
            if let Some(u) = until {
                res["until"] = json!(u);
            }
            if let Some(sf) = show_files {
                res["show_files"] = json!(sf);
            }
            if let Some(ol) = oneline {
                res["oneline"] = json!(ol);
            }
            if let Some(fmt) = format {
                res["format"] = json!(fmt);
            }
            res
        }
        _ => args.clone(),
    }
}

/// Normalizes both tool name and arguments from any external convention into Fusion canonical form.
pub fn normalize_tool_call(name: &str, args: &Value) -> (String, Value) {
    let canonical = canonical_tool_name(name);
    let normalized_args = normalize_tool_args(canonical, args);
    (canonical.to_string(), normalized_args)
}

// ===========================================================================
// StrReplaceEditorTool (Anthropic Claude / Agent Convention)
// ===========================================================================

/// Anthropic Claude standard `str_replace_editor` tool compatibility implementation.
/// Supports commands: `view`, `create`, `str_replace`, `insert`, `undo_edit`.
#[derive(Default, Debug, Clone)]
pub struct StrReplaceEditorTool;

impl StrReplaceEditorTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for StrReplaceEditorTool {
    fn name(&self) -> &str {
        "str_replace_editor"
    }

    fn description(&self) -> &str {
        "Custom editing tool for viewing, creating, and editing files. Supports commands: 'view', 'create', 'str_replace', 'insert', 'undo_edit'."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "enum": ["view", "create", "str_replace", "insert", "undo_edit"],
                    "description": "The command to run. Allowed options are: `view`, `create`, `str_replace`, `insert`, `undo_edit`."
                },
                "path": {
                    "type": "string",
                    "description": "The path to the file (relative or absolute)."
                },
                "file_text": {
                    "type": "string",
                    "description": "Required parameter of `create` command, with the content of the file to be created."
                },
                "old_str": {
                    "type": "string",
                    "description": "Required parameter of `str_replace` command containing the string in `path` to replace."
                },
                "new_str": {
                    "type": "string",
                    "description": "Optional parameter of `str_replace` command containing the new string. Required parameter of `insert` command containing the string to insert."
                },
                "insert_line": {
                    "type": "integer",
                    "description": "Required parameter of `insert` command. The `new_str` will be inserted AFTER the line `insert_line` of `path`."
                },
                "view_range": {
                    "type": "array",
                    "items": { "type": "integer" },
                    "description": "Optional parameter of `view` command when `path` points to a file. The [start_line, end_line] 1-based range to view."
                }
            },
            "required": ["command", "path"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> anyhow::Result<String> {
        let path_str = extract_string_field(
            &args,
            &["path", "file_path", "filePath", "file", "filename", "target_file"],
        )
        .ok_or_else(|| anyhow::anyhow!("Missing required parameter: path"))?;

        let command = extract_string_field(&args, &["command", "action", "cmd", "mode"]);

        match command {
            Some("view") => {
                let mut read_args = json!({ "path": path_str });
                if let Some(range) = args.get("view_range").and_then(|v| v.as_array()) {
                    if !range.is_empty() {
                        if let Some(start) = range.first().and_then(|v| v.as_u64()) {
                            read_args["offset"] = json!(start);
                            if let Some(end) = range.get(1).and_then(|v| v.as_u64()) {
                                if end >= start {
                                    read_args["limit"] = json!(end - start + 1);
                                }
                            }
                        }
                    }
                }
                ReadFileTool::new().execute(read_args, ctx).await
            }
            Some("create") => {
                let content = extract_string_field(
                    &args,
                    &["file_text", "content", "text", "contents", "body", "data"],
                )
                .unwrap_or_default();

                let write_args = json!({
                    "path": path_str,
                    "content": content,
                });
                WriteFileTool::new().execute(write_args, ctx).await
            }
            Some("str_replace") => {
                let old_str = extract_string_field(
                    &args,
                    &["old_str", "old_text", "oldText", "search", "find", "original_text"],
                )
                .ok_or_else(|| anyhow::anyhow!("Missing required parameter for str_replace: old_str"))?;

                let new_str = extract_string_field(
                    &args,
                    &["new_str", "new_text", "newText", "replace", "replacement"],
                )
                .unwrap_or_default();

                let edit_args = json!({
                    "path": path_str,
                    "old_text": old_str,
                    "new_text": new_str,
                });
                EditFileTool::new().execute(edit_args, ctx).await
            }
            Some("insert") => {
                let new_str = extract_string_field(&args, &["new_str", "content", "text", "new_text"])
                    .ok_or_else(|| anyhow::anyhow!("Missing required parameter for insert: new_str"))?;

                let insert_line = extract_u64_field(&args, &["insert_line", "line", "line_number", "after_line"])
                    .unwrap_or(0) as usize;

                let full_path = resolve_path(path_str, &ctx.cwd);
                if !full_path.exists() {
                    anyhow::bail!("File not found: '{}'", full_path.display());
                }

                let content = tokio::fs::read_to_string(&full_path)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to read file '{}': {e}", full_path.display()))?;

                let mut lines: Vec<&str> = content.lines().collect();
                let had_trailing_newline = content.ends_with('\n');

                let target_pos = insert_line.min(lines.len());
                let insert_lines: Vec<&str> = new_str.lines().collect();

                let mut new_lines = Vec::with_capacity(lines.len() + insert_lines.len());
                new_lines.extend_from_slice(&lines[..target_pos]);
                new_lines.extend_from_slice(&insert_lines);
                new_lines.extend_from_slice(&lines[target_pos..]);

                let mut result_content = new_lines.join("\n");
                if had_trailing_newline || new_str.ends_with('\n') {
                    result_content.push('\n');
                }

                tokio::fs::write(&full_path, result_content.as_bytes())
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to write file '{}': {e}", full_path.display()))?;

                Ok(format!(
                    "Successfully inserted {} line(s) after line {} in '{}'",
                    insert_lines.len().max(1),
                    insert_line,
                    path_str
                ))
            }
            Some("undo_edit") => {
                Ok(format!(
                    "Undo requested for '{}'. Note: Undo is managed via session checkpoints (/rewind) or git history.",
                    path_str
                ))
            }
            _ => {
                // Infer command from parameters if no command given
                if let Some(old_str) = extract_string_field(&args, &["old_str", "old_text", "search", "find"]) {
                    let new_str = extract_string_field(&args, &["new_str", "new_text", "replace"]).unwrap_or_default();
                    let edit_args = json!({
                        "path": path_str,
                        "old_text": old_str,
                        "new_text": new_str,
                    });
                    EditFileTool::new().execute(edit_args, ctx).await
                } else if let Some(content) = extract_string_field(&args, &["file_text", "content", "text"]) {
                    let write_args = json!({
                        "path": path_str,
                        "content": content,
                    });
                    WriteFileTool::new().execute(write_args, ctx).await
                } else {
                    let mut read_args = json!({ "path": path_str });
                    if let Some(range) = args.get("view_range").and_then(|v| v.as_array()) {
                        if !range.is_empty() {
                            if let Some(start) = range.first().and_then(|v| v.as_u64()) {
                                read_args["offset"] = json!(start);
                                if let Some(end) = range.get(1).and_then(|v| v.as_u64()) {
                                    if end >= start {
                                        read_args["limit"] = json!(end - start + 1);
                                    }
                                }
                            }
                        }
                    }
                    ReadFileTool::new().execute(read_args, ctx).await
                }
            }
        }
    }
}

// ===========================================================================
// ViewTool (Agent Convention -> Read)
// ===========================================================================

/// Compatibility tool mapping `view` to `read` with support for `view_range` and line offsets.
#[derive(Default, Debug, Clone)]
pub struct ViewTool;

impl ViewTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for ViewTool {
    fn name(&self) -> &str {
        "view"
    }

    fn description(&self) -> &str {
        "View and inspect file content with optional line offsets, limits, or view ranges (alias for read)."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to view (relative or absolute)."
                },
                "view_range": {
                    "type": "array",
                    "items": { "type": "integer" },
                    "description": "Optional [start_line, end_line] 1-based range to view."
                },
                "offset": {
                    "type": "integer",
                    "description": "1-based line number to start reading from."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of lines to read."
                },
                "line_numbers": {
                    "type": "boolean",
                    "description": "Whether to prefix output lines with line numbers."
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> anyhow::Result<String> {
        let normalized = normalize_tool_args("read", &args);
        ReadFileTool::new().execute(normalized, ctx).await
    }
}

// ===========================================================================
// CatTool (CLI / Unix Convention -> Read)
// ===========================================================================

/// Compatibility tool mapping `cat` to `read`.
#[derive(Default, Debug, Clone)]
pub struct CatTool;

impl CatTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for CatTool {
    fn name(&self) -> &str {
        "cat"
    }

    fn description(&self) -> &str {
        "Print and inspect file content (alias for read)."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to display."
                },
                "offset": {
                    "type": "integer",
                    "description": "1-based line number to start reading from."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of lines to read."
                },
                "line_numbers": {
                    "type": "boolean",
                    "description": "Whether to prefix lines with line numbers."
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> anyhow::Result<String> {
        let normalized = normalize_tool_args("read", &args);
        ReadFileTool::new().execute(normalized, ctx).await
    }
}

// ===========================================================================
// CreateTool (Agent Convention -> Write)
// ===========================================================================

/// Compatibility tool mapping `create` to `write`.
#[derive(Default, Debug, Clone)]
pub struct CreateTool;

impl CreateTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for CreateTool {
    fn name(&self) -> &str {
        "create"
    }

    fn description(&self) -> &str {
        "Create a new file or overwrite an existing file with content (alias for write)."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to create or overwrite."
                },
                "content": {
                    "type": "string",
                    "description": "Content to write into the file."
                },
                "file_text": {
                    "type": "string",
                    "description": "Alternative content parameter for compatibility."
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> anyhow::Result<String> {
        let normalized = normalize_tool_args("write", &args);
        WriteFileTool::new().execute(normalized, ctx).await
    }
}

// ===========================================================================
// TerminalTool (Agent Convention -> Bash)
// ===========================================================================

/// Compatibility tool mapping `terminal` to `bash`.
#[derive(Default, Debug, Clone)]
pub struct TerminalTool;

impl TerminalTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for TerminalTool {
    fn name(&self) -> &str {
        "terminal"
    }

    fn description(&self) -> &str {
        "Execute commands in a terminal environment (alias for bash)."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The shell command to execute."
                },
                "cmd": {
                    "type": "string",
                    "description": "Alternative key for command."
                },
                "timeout": {
                    "type": "integer",
                    "description": "Execution timeout in seconds (default: 30)."
                },
                "cwd": {
                    "type": "string",
                    "description": "Working directory for command execution."
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> anyhow::Result<String> {
        let normalized = normalize_tool_args("bash", &args);
        BashTool::new().execute(normalized, ctx).await
    }
}

// ===========================================================================
// Generic ToolAlias Wrapper
// ===========================================================================

/// A generic wrapper struct allowing any `DynTool` to be registered under an alias name.
pub struct ToolAlias {
    alias_name: String,
    description: String,
    canonical_target: String,
    inner: DynTool,
}

impl ToolAlias {
    pub fn new(alias_name: impl Into<String>, canonical_target: impl Into<String>, inner: DynTool) -> Self {
        let alias = alias_name.into();
        let target = canonical_target.into();
        let desc = format!("Alias for '{}': {}", target, inner.description());
        Self {
            alias_name: alias,
            description: desc,
            canonical_target: target,
            inner,
        }
    }
}

#[async_trait]
impl Tool for ToolAlias {
    fn name(&self) -> &str {
        &self.alias_name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters(&self) -> Value {
        self.inner.parameters()
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.alias_name.clone(),
            description: self.description.clone(),
            parameters: self.parameters(),
        }
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> anyhow::Result<String> {
        let normalized = normalize_tool_args(&self.canonical_target, &args);
        self.inner.execute(normalized, ctx).await
    }
}

// ===========================================================================
// Registration Helpers
// ===========================================================================

/// Returns instances of all core compatibility tools.
pub fn all_compat_tools() -> Vec<DynTool> {
    vec![
        Arc::new(StrReplaceEditorTool::new()),
        Arc::new(ViewTool::new()),
        Arc::new(CatTool::new()),
        Arc::new(CreateTool::new()),
        Arc::new(TerminalTool::new()),
    ]
}

/// Registers all compatibility tools and aliases into a given `ToolRegistry`.
pub fn register_compat_tools(registry: &mut ToolRegistry) {
    for tool in all_compat_tools() {
        registry.register(tool);
    }
}

/// Creates a new `ToolRegistry` containing standard tools plus all compatibility tools.
pub fn compat_registry() -> ToolRegistry {
    let mut registry = crate::tools::default_registry();
    register_compat_tools(&mut registry);
    registry
}

// ===========================================================================
// Unit Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::path::PathBuf;

    static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn temp_test_dir() -> PathBuf {
        let count = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "fusion_compat_test_{}_{}_{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0),
            count
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_canonical_name_mapping() {
        assert_eq!(canonical_tool_name("str_replace_editor"), "edit");
        assert_eq!(canonical_tool_name("str_replace"), "edit");
        assert_eq!(canonical_tool_name("view"), "read");
        assert_eq!(canonical_tool_name("cat"), "read");
        assert_eq!(canonical_tool_name("create"), "write");
        assert_eq!(canonical_tool_name("create_file"), "write");
        assert_eq!(canonical_tool_name("terminal"), "bash");
        assert_eq!(canonical_tool_name("shell"), "bash");
        assert_eq!(canonical_tool_name("exec"), "bash");
        assert_eq!(canonical_tool_name("search"), "grep");
        assert_eq!(canonical_tool_name("find_files"), "glob");
        assert_eq!(canonical_tool_name("custom_tool"), "custom_tool");
    }

    #[test]
    fn test_is_compat_alias() {
        assert!(is_compat_alias("str_replace_editor"));
        assert!(is_compat_alias("view"));
        assert!(is_compat_alias("cat"));
        assert!(is_compat_alias("create"));
        assert!(is_compat_alias("terminal"));
        assert!(!is_compat_alias("unregistered_random_tool"));
    }

    #[test]
    fn test_normalize_tool_call_edit() {
        let args = json!({
            "filePath": "src/main.rs",
            "old_str": "fn foo() {}",
            "new_str": "fn bar() {}"
        });
        let (name, norm) = normalize_tool_call("str_replace_editor", &args);
        assert_eq!(name, "edit");
        assert_eq!(norm["path"], "src/main.rs");
        assert_eq!(norm["old_text"], "fn foo() {}");
        assert_eq!(norm["new_text"], "fn bar() {}");
    }

    #[test]
    fn test_normalize_tool_call_view_range() {
        let args = json!({
            "file": "README.md",
            "view_range": [10, 25]
        });
        let (name, norm) = normalize_tool_call("view", &args);
        assert_eq!(name, "read");
        assert_eq!(norm["path"], "README.md");
        assert_eq!(norm["offset"], 10);
        assert_eq!(norm["limit"], 16);
    }

    #[test]
    fn test_normalize_tool_call_create() {
        let args = json!({
            "file_path": "test.txt",
            "file_text": "hello world"
        });
        let (name, norm) = normalize_tool_call("create", &args);
        assert_eq!(name, "write");
        assert_eq!(norm["path"], "test.txt");
        assert_eq!(norm["content"], "hello world");
    }

    #[test]
    fn test_normalize_tool_call_terminal() {
        let args = json!({
            "cmd": "echo 'hi'",
            "dir": "/tmp"
        });
        let (name, norm) = normalize_tool_call("terminal", &args);
        assert_eq!(name, "bash");
        assert_eq!(norm["command"], "echo 'hi'");
        assert_eq!(norm["cwd"], "/tmp");
    }

    #[test]
    fn test_normalize_tool_call_git_log() {
        let args = json!({
            "limit": 5,
            "author": "Alice",
            "pattern": "fix bug",
            "file": "src/main.rs"
        });
        let (name, norm) = normalize_tool_call("git_history", &args);
        assert_eq!(name, "git_log");
        assert_eq!(norm["max_count"], 5);
        assert_eq!(norm["author"], "Alice");
        assert_eq!(norm["grep"], "fix bug");
        assert_eq!(norm["file_path"], "src/main.rs");
    }

    #[tokio::test]
    async fn test_create_and_view_compat_tools() {
        let dir = temp_test_dir();
        let ctx = ToolContext {
            cwd: dir.clone(),
            env: std::collections::HashMap::new(),
        };

        let create_tool = CreateTool::new();
        let view_tool = ViewTool::new();

        // 1. Create file using `create`
        let create_res = create_tool
            .execute(
                json!({
                    "path": "hello.txt",
                    "file_text": "Line 1\nLine 2\nLine 3\nLine 4\nLine 5"
                }),
                &ctx,
            )
            .await;
        assert!(create_res.is_ok());

        // 2. View file using `view` with view_range
        let view_res = view_tool
            .execute(
                json!({
                    "path": "hello.txt",
                    "view_range": [2, 4],
                    "line_numbers": false
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(view_res.contains("Line 2"));
        assert!(view_res.contains("Line 3"));
        assert!(view_res.contains("Line 4"));
        assert!(!view_res.contains("Line 1"));
    }

    #[tokio::test]
    async fn test_cat_compat_tool() {
        let dir = temp_test_dir();
        let ctx = ToolContext {
            cwd: dir.clone(),
            env: std::collections::HashMap::new(),
        };

        let test_file = dir.join("cat_test.txt");
        std::fs::write(&test_file, "Cat test content\nSecond line").unwrap();

        let cat_tool = CatTool::new();
        let res = cat_tool
            .execute(json!({ "path": "cat_test.txt" }), &ctx)
            .await
            .unwrap();

        assert!(res.contains("Cat test content"));
        assert!(res.contains("Second line"));
    }

    #[tokio::test]
    async fn test_str_replace_editor_all_commands() {
        let dir = temp_test_dir();
        let ctx = ToolContext {
            cwd: dir.clone(),
            env: std::collections::HashMap::new(),
        };

        let editor = StrReplaceEditorTool::new();

        // 1. create command
        let create_res = editor
            .execute(
                json!({
                    "command": "create",
                    "path": "editor_test.rs",
                    "file_text": "fn main() {\n    println!(\"Hello\");\n}\n"
                }),
                &ctx,
            )
            .await;
        assert!(create_res.is_ok());

        // 2. view command
        let view_res = editor
            .execute(
                json!({
                    "command": "view",
                    "path": "editor_test.rs",
                    "view_range": [1, 2]
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(view_res.contains("fn main()"));

        // 3. str_replace command
        let edit_res = editor
            .execute(
                json!({
                    "command": "str_replace",
                    "path": "editor_test.rs",
                    "old_str": "println!(\"Hello\");",
                    "new_str": "println!(\"World\");"
                }),
                &ctx,
            )
            .await;
        assert!(edit_res.is_ok());

        let content = std::fs::read_to_string(dir.join("editor_test.rs")).unwrap();
        assert!(content.contains("println!(\"World\");"));
        assert!(!content.contains("println!(\"Hello\");"));

        // 4. insert command
        let insert_res = editor
            .execute(
                json!({
                    "command": "insert",
                    "path": "editor_test.rs",
                    "insert_line": 2,
                    "new_str": "    // added comment"
                }),
                &ctx,
            )
            .await;
        assert!(insert_res.is_ok());

        let updated = std::fs::read_to_string(dir.join("editor_test.rs")).unwrap();
        assert!(updated.contains("// added comment"));

        // 5. undo_edit command
        let undo_res = editor
            .execute(
                json!({
                    "command": "undo_edit",
                    "path": "editor_test.rs"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(undo_res.contains("Undo requested"));
    }

    #[tokio::test]
    async fn test_terminal_compat_tool() {
        let dir = temp_test_dir();
        let ctx = ToolContext {
            cwd: dir.clone(),
            env: std::collections::HashMap::new(),
        };

        let term_tool = TerminalTool::new();
        let res = term_tool
            .execute(json!({ "cmd": "echo 'compat_terminal_ok'" }), &ctx)
            .await
            .unwrap();

        assert!(res.contains("compat_terminal_ok"));
    }

    #[tokio::test]
    async fn test_registry_with_compat_tools() {
        let dir = temp_test_dir();
        let ctx = ToolContext {
            cwd: dir.clone(),
            env: std::collections::HashMap::new(),
        };

        let reg = compat_registry();

        assert!(reg.contains("str_replace_editor"));
        assert!(reg.contains("view"));
        assert!(reg.contains("cat"));
        assert!(reg.contains("create"));
        assert!(reg.contains("terminal"));

        // Execute create via registry
        let write_res = reg
            .execute(
                "create",
                json!({ "path": "from_reg.txt", "file_text": "via_registry" }),
                &ctx,
            )
            .await;
        assert!(write_res.is_ok());

        // Execute view via registry
        let view_res = reg
            .execute("view", json!({ "path": "from_reg.txt" }), &ctx)
            .await
            .unwrap();
        assert!(view_res.contains("via_registry"));
    }

    #[test]
    fn test_normalize_path_arg_variants() {
        // Windows backslashes -> forward slashes
        assert_eq!(normalize_path_arg("src\\lib\\main.rs"), "src/lib/main.rs");
        // POSIX untouched
        assert_eq!(normalize_path_arg("src/lib/main.rs"), "src/lib/main.rs");
        // Trimming whitespace
        assert_eq!(normalize_path_arg("  src/main.rs  "), "src/main.rs");
        // Surrounding quotes stripped
        assert_eq!(normalize_path_arg("\"my dir/file.rs\""), "my dir/file.rs");
        // Windows verbatim prefix preserved untouched
        assert_eq!(normalize_path_arg("\\\\?\\C:\\very\\long\\path"), "\\\\?\\C:\\very\\long\\path");
        // Windows device prefix preserved untouched
        assert_eq!(normalize_path_arg("\\\\.\\PhysicalDrive0"), "\\\\.\\PhysicalDrive0");
        // UNC share preserved untouched
        assert_eq!(normalize_path_arg("\\\\server\\share\\file.txt"), "\\\\server\\share\\file.txt");
        // Empty string stays empty
        assert_eq!(normalize_path_arg(""), "");
    }

    #[test]
    fn test_extract_path_field_normalizes() {
        let args = json!({ "file_path": "src\\nested\\module.rs" });
        assert_eq!(
            extract_path_field(&args, &["path", "file_path"]),
            Some("src/nested/module.rs".to_string())
        );
        // Falls back through the key list
        let args = json!({ "filename": "a\\b.txt" });
        assert_eq!(
            extract_path_field(&args, &["path", "file_path", "filename"]),
            Some("a/b.txt".to_string())
        );
        // Missing key -> None
        assert_eq!(extract_path_field(&args, &["nope"]), None);
    }

    #[test]
    fn test_normalize_tool_args_edit_windows_path() {
        let args = json!({
            "filePath": "src\\win\\main.rs",
            "old_str": "fn foo() {}",
            "new_str": "fn bar() {}"
        });
        let norm = normalize_tool_args("edit", &args);
        assert_eq!(norm["path"], "src/win/main.rs");
    }

    #[test]
    fn test_normalize_tool_args_read_windows_path() {
        let args = json!({ "file": "docs\\guide\\intro.md" });
        let norm = normalize_tool_args("read", &args);
        assert_eq!(norm["path"], "docs/guide/intro.md");
    }

    #[test]
    fn test_normalize_tool_args_write_windows_path() {
        let args = json!({
            "dest": "out\\build\\artifact.bin",
            "data": "payload"
        });
        let norm = normalize_tool_args("write", &args);
        assert_eq!(norm["path"], "out/build/artifact.bin");
        assert_eq!(norm["content"], "payload");
    }

    #[test]
    fn test_normalize_tool_args_bash_cwd_windows_path() {
        let args = json!({
            "cmd": "dir",
            "workdir": "C:\\project\\src"
        });
        let norm = normalize_tool_args("bash", &args);
        assert_eq!(norm["command"], "dir");
        // Drive prefix path normalized to forward slashes
        assert_eq!(norm["cwd"], "C:/project/src");
    }

    #[test]
    fn test_normalize_tool_args_unknown_tool_passthrough() {
        let args = json!({ "custom": "value", "path": "src\\x.rs" });
        let (name, norm) = normalize_tool_call("totally_custom", &args);
        assert_eq!(name, "totally_custom");
        // Unknown tools pass args through untouched
        assert_eq!(norm, args);
    }

    #[test]
    fn test_normalize_tool_call_git_log_path() {
        let args = json!({ "repo_path": "vendor\\lib", "limit": 3 });
        let (name, norm) = normalize_tool_call("gitlog", &args);
        assert_eq!(name, "git_log");
        assert_eq!(norm["path"], "vendor/lib");
        assert_eq!(norm["max_count"], 3);
    }
}
