//! Language Server Protocol (LSP) tool for semantic code intelligence.
//!
//! Provides fast semantic navigation and diagnostics via LSP servers (rust-analyzer,
//! typescript-language-server, pyright, gopls, clangd, etc.) with automatic graceful
//! fallback to regex-based symbol scanning and syntax validation when language servers
//! are unavailable.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::tools::file::resolve_path;
use crate::tools::lsp::client::{path_to_uri, LspClient};
use crate::tools::lsp::config::{find_server_for_file, LspServerDef};
use crate::tools::symbols::SymbolsTool;
use crate::tools::syntax::SyntaxCheckTool;
use crate::tools::types::{Tool, ToolContext};

/// Tool for querying Language Server Protocol servers for code intelligence.
#[derive(Default, Debug, Clone)]
pub struct LspTool;

impl LspTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for LspTool {
    fn name(&self) -> &str {
        "lsp"
    }

    fn description(&self) -> &str {
        "Query language servers (rust-analyzer, vtsls, pyright, gopls, clangd, etc.) for semantic code intelligence: definition, references, type_definition, diagnostics, and symbols."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["definition", "references", "type_definition", "diagnostics", "symbols"],
                    "description": "LSP action: 'definition', 'references', 'type_definition', 'diagnostics', or 'symbols'."
                },
                "file": {
                    "type": "string",
                    "description": "File path (or '*' for workspace diagnostics)."
                },
                "line": {
                    "type": "integer",
                    "description": "1-indexed line number."
                },
                "character": {
                    "type": "integer",
                    "description": "Optional 1-indexed character / column position (defaults to 1)."
                },
                "symbol": {
                    "type": "string",
                    "description": "Optional symbol name for symbol lookup."
                }
            },
            "required": ["action", "file"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> anyhow::Result<String> {
        let action = match args.get("action").and_then(|v| v.as_str()) {
            Some(a) => a.trim().to_lowercase(),
            None => {
                anyhow::bail!(
                    "Missing required parameter: 'action'. Supported actions: definition, references, type_definition, diagnostics, symbols"
                );
            }
        };

        let file_str = match args
            .get("file")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("path").and_then(|v| v.as_str()))
        {
            Some(f) => f.trim(),
            None => anyhow::bail!("Missing required parameter: 'file'."),
        };

        let line_1 = args.get("line").and_then(|v| v.as_u64()).unwrap_or(1);
        let char_1 = args
            .get("character")
            .or_else(|| args.get("column"))
            .or_else(|| args.get("col"))
            .and_then(|v| v.as_u64())
            .unwrap_or(1);

        let line_0 = if line_1 > 0 { (line_1 - 1) as u32 } else { 0 };
        let char_0 = if char_1 > 0 { (char_1 - 1) as u32 } else { 0 };

        let explicit_symbol = args
            .get("symbol")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let resolved_path = if file_str == "*" {
            ctx.cwd.clone()
        } else {
            resolve_path(file_str, &ctx.cwd)
        };

        let server_opt = if file_str == "*" {
            find_server_for_workspace(&ctx.cwd)
        } else {
            find_server_for_file(&resolved_path)
        };

        let binary_available = server_opt
            .as_ref()
            .map_or(false, |s| is_binary_on_path(&s.command));

        if let Some(server_def) = &server_opt {
            if binary_available {
                match execute_with_lsp(
                    server_def,
                    &action,
                    &resolved_path,
                    file_str,
                    line_0,
                    char_0,
                    explicit_symbol.as_deref(),
                    ctx,
                )
                .await
                {
                    Ok(resp) => return Ok(resp),
                    Err(err) => {
                        return fallback_handler(
                            &action,
                            &resolved_path,
                            file_str,
                            line_1,
                            char_1,
                            explicit_symbol.as_deref(),
                            server_opt.as_ref(),
                            Some(&err.to_string()),
                            ctx,
                        )
                        .await;
                    }
                }
            }
        }

        fallback_handler(
            &action,
            &resolved_path,
            file_str,
            line_1,
            char_1,
            explicit_symbol.as_deref(),
            server_opt.as_ref(),
            None,
            ctx,
        )
        .await
    }
}

/// Attempts LSP interaction within a bounded timeout.
async fn execute_with_lsp(
    server_def: &LspServerDef,
    action: &str,
    resolved_path: &Path,
    file_str: &str,
    line_0: u32,
    char_0: u32,
    symbol: Option<&str>,
    ctx: &ToolContext,
) -> anyhow::Result<String> {
    tokio::time::timeout(Duration::from_secs(12), async {
        let client = LspClient::new(&server_def.command, &server_def.args)?;
        let root_dir = find_root_dir(resolved_path, &server_def.root_markers, &ctx.cwd);
        let _ = client.initialize(&root_dir).await?;
        let _ = client.initialized().await;

        if resolved_path.is_file() {
            if let Ok(content) = tokio::fs::read_to_string(resolved_path).await {
                let lang_id = detect_language_id(resolved_path);
                let _ = client.did_open(resolved_path, lang_id, &content).await;
            }
        }

        let result = match action {
            "definition" => {
                let locs = client
                    .goto_definition(resolved_path, line_0, char_0)
                    .await?;
                if locs.is_empty() {
                    Ok(format!(
                        "No LSP definitions found at {}:{}:{}.",
                        file_str,
                        line_0 + 1,
                        char_0 + 1
                    ))
                } else {
                    Ok(format_locations(&locs, &ctx.cwd))
                }
            }
            "references" => {
                let locs = client
                    .find_references(resolved_path, line_0, char_0)
                    .await?;
                if locs.is_empty() {
                    Ok(format!(
                        "No LSP references found at {}:{}:{}.",
                        file_str,
                        line_0 + 1,
                        char_0 + 1
                    ))
                } else {
                    Ok(format_locations(&locs, &ctx.cwd))
                }
            }
            "type_definition" => {
                let uri = path_to_uri(resolved_path);
                let params = json!({
                    "textDocument": { "uri": uri },
                    "position": { "line": line_0, "character": char_0 }
                });
                let res = client
                    .send_request("textDocument/typeDefinition", params)
                    .await?;
                let locs = parse_locations_response(res);
                if locs.is_empty() {
                    Ok(format!(
                        "No LSP type definitions found at {}:{}:{}.",
                        file_str,
                        line_0 + 1,
                        char_0 + 1
                    ))
                } else {
                    Ok(format_locations(&locs, &ctx.cwd))
                }
            }
            "diagnostics" => {
                let uri = path_to_uri(resolved_path);
                let params = json!({
                    "textDocument": { "uri": uri }
                });
                let res = client
                    .send_request("textDocument/diagnostic", params)
                    .await?;
                Ok(format_lsp_diagnostics(&res, file_str))
            }
            "symbols" => {
                if let Some(sym_query) = symbol {
                    let params = json!({ "query": sym_query });
                    if let Ok(res) = client.send_request("workspace/symbol", params).await {
                        if let Some(arr) = res.as_array() {
                            if !arr.is_empty() {
                                return Ok(format_lsp_symbols(&res));
                            }
                        }
                    }
                }
                let uri = path_to_uri(resolved_path);
                let params = json!({ "textDocument": { "uri": uri } });
                let res = client
                    .send_request("textDocument/documentSymbol", params)
                    .await?;
                Ok(format_lsp_symbols(&res))
            }
            _ => anyhow::bail!("Unsupported action: {}", action),
        };

        let _ = client.shutdown().await;
        result
    })
    .await
    .map_err(|_| anyhow::anyhow!("LSP request timed out"))?
}

/// Graceful fallback handler when LSP server is not installed or encounters an error.
async fn fallback_handler(
    action: &str,
    resolved_path: &Path,
    file_str: &str,
    line_1: u64,
    char_1: u64,
    explicit_symbol: Option<&str>,
    server_opt: Option<&LspServerDef>,
    server_error: Option<&str>,
    ctx: &ToolContext,
) -> anyhow::Result<String> {
    let server_name = server_opt.map(|s| s.command.as_str()).unwrap_or("unknown");
    let reason = if let Some(err) = server_error {
        format!("Language server '{}' failed ({})", server_name, err)
    } else if server_opt.is_some() {
        format!("Language server '{}' not found on PATH", server_name)
    } else {
        format!("No LSP server configured for '{}'", file_str)
    };

    let symbol = explicit_symbol
        .map(|s| s.to_string())
        .or_else(|| extract_symbol_from_file(resolved_path, line_1, char_1));

    match action {
        "definition" => {
            if let Some(sym) = &symbol {
                let sym_tool = SymbolsTool::new();
                let sym_args = json!({
                    "query": sym,
                    "exact": true,
                    "path": if file_str != "*" { file_str } else { "." }
                });
                if let Ok(out) = sym_tool.execute(sym_args, ctx).await {
                    if !out.is_empty() && !out.contains("No symbols found") {
                        return Ok(format!(
                            "[LSP Fallback -> Symbols] {}.\nFound symbol definition(s) for '{}':\n{}",
                            reason, sym, out
                        ));
                    }
                }
                Ok(format!(
                    "[LSP Fallback] {}. No symbol definition found for '{}'.",
                    reason, sym
                ))
            } else {
                Ok(format!(
                    "[LSP Fallback] {}. No symbol specified or found at line {}.",
                    reason, line_1
                ))
            }
        }
        "references" => {
            if let Some(sym) = &symbol {
                let sym_tool = SymbolsTool::new();
                let sym_args = json!({
                    "query": sym,
                    "path": if file_str != "*" { file_str } else { "." }
                });
                if let Ok(out) = sym_tool.execute(sym_args, ctx).await {
                    if !out.is_empty() && !out.contains("No symbols found") {
                        return Ok(format!(
                            "[LSP Fallback -> Symbols] {}.\nReferences for '{}':\n{}",
                            reason, sym, out
                        ));
                    }
                }
                Ok(format!(
                    "[LSP Fallback] {}. No references found for '{}'.",
                    reason, sym
                ))
            } else {
                Ok(format!(
                    "[LSP Fallback] {}. No symbol specified for references lookup.",
                    reason
                ))
            }
        }
        "type_definition" => {
            if let Some(sym) = &symbol {
                let sym_tool = SymbolsTool::new();
                let sym_args = json!({
                    "query": sym,
                    "kind": "type",
                    "path": if file_str != "*" { file_str } else { "." }
                });
                if let Ok(out) = sym_tool.execute(sym_args, ctx).await {
                    if !out.is_empty() && !out.contains("No symbols found") {
                        return Ok(format!(
                            "[LSP Fallback -> Symbols] {}.\nType definition for '{}':\n{}",
                            reason, sym, out
                        ));
                    }
                }
                // Fallback to exact symbol search
                let sym_args_all = json!({
                    "query": sym,
                    "exact": true,
                    "path": if file_str != "*" { file_str } else { "." }
                });
                if let Ok(out) = sym_tool.execute(sym_args_all, ctx).await {
                    if !out.is_empty() && !out.contains("No symbols found") {
                        return Ok(format!(
                            "[LSP Fallback -> Symbols] {}.\nType definition for '{}':\n{}",
                            reason, sym, out
                        ));
                    }
                }
                Ok(format!(
                    "[LSP Fallback] {}. No type definition found for '{}'.",
                    reason, sym
                ))
            } else {
                Ok(format!(
                    "[LSP Fallback] {}. No symbol specified for type definition lookup.",
                    reason
                ))
            }
        }
        "symbols" => {
            let sym_tool = SymbolsTool::new();
            let mut sym_args = json!({});
            if let Some(sym) = &symbol {
                sym_args["query"] = json!(sym);
            }
            if file_str != "*" {
                sym_args["path"] = json!(file_str);
            }
            match sym_tool.execute(sym_args, ctx).await {
                Ok(out) => Ok(format!("[LSP Fallback -> Symbols] {}.\n{}", reason, out)),
                Err(e) => Ok(format!(
                    "[LSP Fallback] {}. Symbols scanner error: {}",
                    reason, e
                )),
            }
        }
        "diagnostics" => {
            if file_str != "*" && resolved_path.is_file() {
                let syntax_tool = SyntaxCheckTool::new();
                let syn_args = json!({ "path": file_str });
                match syntax_tool.execute(syn_args, ctx).await {
                    Ok(out) => Ok(format!("[LSP Fallback -> Syntax] {}.\n{}", reason, out)),
                    Err(e) => Ok(format!(
                        "[LSP Fallback] {}. Syntax checker error: {}",
                        reason, e
                    )),
                }
            } else {
                Ok(format!(
                    "[LSP Fallback] {}. Diagnostics require a specific file or active language server.",
                    reason
                ))
            }
        }
        _ => anyhow::bail!("Unsupported action: {}", action),
    }
}

/// Resolves the project root directory containing markers such as `Cargo.toml` or `.git`.
fn find_root_dir(start: &Path, markers: &[String], default: &Path) -> PathBuf {
    let mut curr = if start.is_file() {
        start.parent()
    } else {
        Some(start)
    };
    while let Some(dir) = curr {
        for marker in markers {
            if dir.join(marker).exists() {
                return dir.to_path_buf();
            }
        }
        if dir.join(".git").exists() {
            return dir.to_path_buf();
        }
        curr = dir.parent();
    }
    default.to_path_buf()
}

/// Detects language server for workspace by matching root markers in workspace root.
fn find_server_for_workspace(root: &Path) -> Option<LspServerDef> {
    let servers = crate::tools::lsp::config::get_default_servers();
    for server_name in crate::tools::lsp::config::DEFAULT_SERVER_ORDER {
        if let Some(server) = servers.get(*server_name) {
            if server.root_markers.iter().any(|m| root.join(m).exists()) {
                return Some(server.clone());
            }
        }
    }
    None
}

/// Checks whether a command binary exists on the system PATH or as an absolute path.
fn is_binary_on_path(command: &str) -> bool {
    let p = Path::new(command);
    if p.is_absolute() || command.contains(std::path::MAIN_SEPARATOR) || command.contains('/') {
        return p.is_file();
    }
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            let candidate = dir.join(command);
            if candidate.is_file() {
                return true;
            }
            #[cfg(windows)]
            {
                for ext in &[".exe", ".cmd", ".bat"] {
                    let c = dir.join(format!("{}{}", command, ext));
                    if c.is_file() {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// Formats a list of LSP Locations or LocationLinks.
pub fn format_locations(locations: &[Value], cwd: &Path) -> String {
    if locations.is_empty() {
        return "No locations found.".to_string();
    }
    let mut out = String::new();
    for loc in locations {
        let uri = loc
            .get("uri")
            .or_else(|| loc.get("targetUri"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let range = loc
            .get("range")
            .or_else(|| loc.get("targetSelectionRange"))
            .or_else(|| loc.get("targetRange"));
        let start = range.and_then(|r| r.get("start"));
        let line_0 = start
            .and_then(|s| s.get("line"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let char_0 = start
            .and_then(|s| s.get("character"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let line = line_0 + 1;
        let col = char_0 + 1;

        let file_path = uri_to_path_buf(uri);
        let rel_path = file_path
            .as_ref()
            .and_then(|p| p.strip_prefix(cwd).ok())
            .map(|p| p.to_string_lossy().to_string())
            .or_else(|| file_path.as_ref().map(|p| p.to_string_lossy().to_string()))
            .unwrap_or_else(|| uri.to_string());

        let line_preview = if let Some(fp) = &file_path {
            if let Ok(content) = std::fs::read_to_string(fp) {
                content
                    .lines()
                    .nth(line_0 as usize)
                    .map(|l| l.trim().to_string())
            } else {
                None
            }
        } else {
            None
        };

        if let Some(preview) = line_preview {
            out.push_str(&format!("{}:{}:{}  {}\n", rel_path, line, col, preview));
        } else {
            out.push_str(&format!("{}:{}:{}\n", rel_path, line, col));
        }
    }
    out.trim_end().to_string()
}

/// Formats LSP symbols hierarchically.
pub fn format_lsp_symbols(val: &Value) -> String {
    let mut out = String::new();
    if let Some(arr) = val.as_array() {
        format_symbols_recursive(arr, 0, &mut out);
    }
    if out.is_empty() {
        "No symbols returned by LSP server.".to_string()
    } else {
        out.trim_end().to_string()
    }
}

fn format_symbols_recursive(arr: &[Value], depth: usize, out: &mut String) {
    let indent = "  ".repeat(depth);
    for item in arr {
        let name = item
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("<unnamed>");
        let kind_num = item.get("kind").and_then(|v| v.as_u64()).unwrap_or(0);
        let kind_name = lsp_symbol_kind_name(kind_num);
        let line = item
            .get("range")
            .or_else(|| item.get("location").and_then(|l| l.get("range")))
            .and_then(|r| r.get("start"))
            .and_then(|s| s.get("line"))
            .and_then(|l| l.as_u64())
            .map(|l| l + 1)
            .unwrap_or(1);

        out.push_str(&format!(
            "{}[{}] {} (line {})\n",
            indent, kind_name, name, line
        ));

        if let Some(children) = item.get("children").and_then(|c| c.as_array()) {
            format_symbols_recursive(children, depth + 1, out);
        }
    }
}

fn lsp_symbol_kind_name(kind: u64) -> &'static str {
    match kind {
        1 => "file",
        2 => "module",
        3 => "namespace",
        4 => "package",
        5 => "class",
        6 => "method",
        7 => "property",
        8 => "field",
        9 => "constructor",
        10 => "enum",
        11 => "interface",
        12 => "function",
        13 => "variable",
        14 => "constant",
        15 => "string",
        16 => "number",
        17 => "boolean",
        18 => "array",
        19 => "object",
        20 => "key",
        21 => "null",
        22 => "enum-member",
        23 => "struct",
        24 => "event",
        25 => "operator",
        26 => "type-param",
        _ => "symbol",
    }
}

/// Formats diagnostics returned by LSP server.
pub fn format_lsp_diagnostics(val: &Value, path_display: &str) -> String {
    let items = val
        .get("items")
        .and_then(|v| v.as_array())
        .or_else(|| val.as_array());

    if let Some(items) = items {
        if items.is_empty() {
            return format!("No diagnostics reported for '{}'.", path_display);
        }
        let mut out = String::new();
        for item in items {
            let msg = item.get("message").and_then(|v| v.as_str()).unwrap_or("");
            let sev_num = item.get("severity").and_then(|v| v.as_u64()).unwrap_or(1);
            let sev_str = match sev_num {
                1 => "error",
                2 => "warning",
                3 => "info",
                4 => "hint",
                _ => "diagnostic",
            };
            let line = item
                .get("range")
                .and_then(|r| r.get("start"))
                .and_then(|s| s.get("line"))
                .and_then(|l| l.as_u64())
                .map(|l| l + 1)
                .unwrap_or(1);
            let col = item
                .get("range")
                .and_then(|r| r.get("start"))
                .and_then(|s| s.get("character"))
                .and_then(|c| c.as_u64())
                .map(|c| c + 1)
                .unwrap_or(1);
            let source = item
                .get("source")
                .and_then(|v| v.as_str())
                .map(|s| format!("[{}] ", s))
                .unwrap_or_default();
            out.push_str(&format!(
                "{}:{}:{}: {}{}: {}\n",
                path_display, line, col, source, sev_str, msg
            ));
        }
        out.trim_end().to_string()
    } else {
        format!("No diagnostics reported for '{}'.", path_display)
    }
}

/// Parses an LSP definition or typeDefinition response into a Vec of Value locations.
pub fn parse_locations_response(res: Value) -> Vec<Value> {
    if res.is_null() {
        Vec::new()
    } else if let Some(arr) = res.as_array() {
        arr.clone()
    } else if res.is_object() {
        vec![res]
    } else {
        Vec::new()
    }
}

/// Converts a `file://` URI to a local `PathBuf`.
pub fn uri_to_path_buf(uri: &str) -> Option<PathBuf> {
    if let Some(stripped) = uri.strip_prefix("file://") {
        #[cfg(windows)]
        {
            let path_str = if stripped.starts_with('/') && stripped.chars().nth(2) == Some(':') {
                &stripped[1..]
            } else {
                stripped
            };
            Some(PathBuf::from(path_str.replace('/', "\\")))
        }
        #[cfg(not(windows))]
        {
            Some(PathBuf::from(stripped))
        }
    } else {
        None
    }
}

/// Extracts symbol identifier from source file at specified line and character position.
pub fn extract_symbol_from_file(path: &Path, line_1: u64, char_1: u64) -> Option<String> {
    if !path.is_file() {
        return None;
    }
    let content = std::fs::read_to_string(path).ok()?;
    let line_0 = if line_1 > 0 { line_1 - 1 } else { 0 } as usize;
    let line_str = content.lines().nth(line_0)?;
    let char_0 = if char_1 > 0 { char_1 - 1 } else { 0 } as usize;
    extract_identifier_at_pos(line_str, char_0)
}

/// Extracts an identifier token around `char_idx` in `line_str`.
pub fn extract_identifier_at_pos(line_str: &str, char_idx: usize) -> Option<String> {
    if line_str.trim().is_empty() {
        return None;
    }
    let chars: Vec<char> = line_str.chars().collect();
    if chars.is_empty() {
        return None;
    }
    let idx = char_idx.min(chars.len() - 1);
    let is_ident_char = |c: char| c.is_alphanumeric() || c == '_';

    let target_idx = if is_ident_char(chars[idx]) {
        idx
    } else if let Some(pos) = (idx..chars.len()).find(|&i| is_ident_char(chars[i])) {
        pos
    } else if let Some(pos) = (0..idx).rfind(|&i| is_ident_char(chars[i])) {
        pos
    } else {
        return None;
    };

    let mut start = target_idx;
    while start > 0 && is_ident_char(chars[start - 1]) {
        start -= 1;
    }
    let mut end = target_idx;
    while end + 1 < chars.len() && is_ident_char(chars[end + 1]) {
        end += 1;
    }

    let ident: String = chars[start..=end].iter().collect();
    if ident.is_empty() {
        None
    } else {
        Some(ident)
    }
}

/// Detects standard LSP language ID from file extension.
fn detect_language_id(path: &Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase());
    match ext.as_deref() {
        Some("rs") => "rust",
        Some("ts") => "typescript",
        Some("tsx") => "typescriptreact",
        Some("js") | Some("mjs") | Some("cjs") => "javascript",
        Some("jsx") => "javascriptreact",
        Some("py") | Some("pyi") => "python",
        Some("go") => "go",
        Some("c") | Some("h") => "c",
        Some("cpp") | Some("hpp") | Some("cc") | Some("cxx") => "cpp",
        Some("json") => "json",
        Some("toml") => "toml",
        Some("yaml") | Some("yml") => "yaml",
        Some("html") | Some("htm") => "html",
        Some("css") => "css",
        Some("scss") => "scss",
        Some("md") | Some("markdown") => "markdown",
        Some("sh") | Some("bash") | Some("zsh") => "shellscript",
        Some("lua") => "lua",
        Some("java") => "java",
        Some("kt") | Some("kts") => "kotlin",
        Some("swift") => "swift",
        Some("zig") => "zig",
        Some("php") => "php",
        Some("rb") => "ruby",
        Some("cs") => "csharp",
        Some("scala") => "scala",
        Some("ex") | Some("exs") => "elixir",
        Some("erl") | Some("hrl") => "erlang",
        Some("hs") => "haskell",
        Some("ml") | Some("mli") => "ocaml",
        Some("dart") => "dart",
        _ => "plaintext",
    }
}
