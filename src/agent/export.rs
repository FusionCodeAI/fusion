//! Session transcript export to self-contained, syntax-highlighted HTML documents.
//!
//! Provides single-file HTML export for persistent agent sessions with:
//! - Modern responsive dark mode styling with optional light mode toggle
//! - Multi-language syntax highlighting for code blocks (Rust, Python, JS/TS, Bash, Go, JSON, SQL, etc.)
//! - Full Markdown rendering (headings, tables, lists, blockquotes, inline formatting)
//! - Collapsible tool calls, tool results, and system prompts
//! - DeepSeek / reasoning model `<think>` block extraction and distinct styling
//! - Interactive client-side features (instant search/filtering, role filter, copy code/message)
//! - Print / PDF-ready styles (`@media print`)
//! - Zero external CDN dependencies (100% self-contained and offline-ready)

use std::fs;
use std::io;
use std::path::Path;

use crate::agent::session::Session;
use crate::provider::types::{Message, Role, ToolCall};

/// Color theme preference for the exported HTML document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExportTheme {
    /// Modern dark theme (default).
    #[default]
    Dark,
    /// Clean light theme.
    Light,
    /// Follow system color scheme preference (`prefers-color-scheme`).
    System,
}

impl ExportTheme {
    /// Returns the CSS attribute or class identifier for the theme.
    pub fn as_attribute_value(&self) -> &'static str {
        match self {
            ExportTheme::Dark => "dark",
            ExportTheme::Light => "light",
            ExportTheme::System => "system",
        }
    }
}

/// Configuration options for tailoring session HTML exports.
#[derive(Debug, Clone)]
pub struct ExportOptions {
    /// Document title override. Defaults to session title or ID.
    pub title: Option<String>,
    /// Theme preference (Dark, Light, or System).
    pub theme: ExportTheme,
    /// Whether to include system messages in the transcript. Default: true.
    pub include_system_messages: bool,
    /// Whether to include tool calls and outputs. Default: true.
    pub include_tool_calls: bool,
    /// Whether to render the metadata/stats header. Default: true.
    pub include_metadata_header: bool,
    /// Whether to syntax highlight code blocks. Default: true.
    pub syntax_highlighting: bool,
    /// Optional custom CSS injected at the bottom of the style tag.
    pub custom_css: Option<String>,
    /// Optional print & PDF stylesheet configuration.
    pub print_options: Option<crate::ui::print_css::PrintOptions>,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            title: None,
            theme: ExportTheme::Dark,
            include_system_messages: true,
            include_tool_calls: true,
            include_metadata_header: true,
            syntax_highlighting: true,
            custom_css: None,
            print_options: None,
        }
    }
}

impl ExportOptions {
    /// Creates a default `ExportOptions` builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets a custom title for the HTML document.
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Sets the color theme.
    pub fn with_theme(mut self, theme: ExportTheme) -> Self {
        self.theme = theme;
        self
    }

    /// Toggles inclusion of system messages.
    pub fn with_system_messages(mut self, include: bool) -> Self {
        self.include_system_messages = include;
        self
    }

    /// Toggles inclusion of tool calls and results.
    pub fn with_tool_calls(mut self, include: bool) -> Self {
        self.include_tool_calls = include;
        self
    }

    /// Toggles the metadata and stats summary header.
    pub fn with_metadata_header(mut self, include: bool) -> Self {
        self.include_metadata_header = include;
        self
    }

    /// Toggles code syntax highlighting.
    pub fn with_syntax_highlighting(mut self, enabled: bool) -> Self {
        self.syntax_highlighting = enabled;
        self
    }

    /// Injects custom CSS rules.
    pub fn with_custom_css(mut self, css: impl Into<String>) -> Self {
        self.custom_css = Some(css.into());
        self
    }

    /// Sets the print and PDF options for the transcript.
    pub fn with_print_options(mut self, print_options: crate::ui::print_css::PrintOptions) -> Self {
        self.print_options = Some(print_options);
        self
    }
}

/// Exports a `Session` to a self-contained HTML string with default dark mode styling.
pub fn export_session_html(session: &Session) -> String {
    export_session_html_with_options(session, &ExportOptions::default())
}

/// Exports a `Session` to a self-contained HTML file on disk.
pub fn export_session_html_file(session: &Session, path: impl AsRef<Path>) -> io::Result<()> {
    export_session_html_file_with_options(session, path, &ExportOptions::default())
}

/// Exports a `Session` to a self-contained HTML file on disk using custom options.
pub fn export_session_html_file_with_options(
    session: &Session,
    path: impl AsRef<Path>,
    options: &ExportOptions,
) -> io::Result<()> {
    let html = export_session_html_with_options(session, options);
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    fs::write(path, html)
}

/// Exports a `Session` to a self-contained HTML string with custom options.
pub fn export_session_html_with_options(session: &Session, options: &ExportOptions) -> String {
    let session_title = options
        .title
        .as_deref()
        .or(session.title.as_deref())
        .unwrap_or("Fusion Session");
    let safe_title = escape_html(session_title);
    let id_str = session.id.to_string();

    let mut html = String::with_capacity(16 * 1024);

    // Document header
    html.push_str("<!DOCTYPE html>\n<html lang=\"en\" data-theme=\"");
    html.push_str(options.theme.as_attribute_value());
    html.push_str("\">\n<head>\n");
    html.push_str("  <meta charset=\"UTF-8\">\n");
    html.push_str("  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\">\n");
    html.push_str("  <title>");
    html.push_str(&safe_title);
    html.push_str(" | Fusion Transcript</title>\n");

    // Embedded Stylesheet
    html.push_str("  <style>\n");
    html.push_str(EMBEDDED_CSS);
    if let Some(print_opts) = &options.print_options {
        html.push_str("\n/* Print & PDF Stylesheet */\n");
        html.push_str(&crate::ui::print_css::generate_print_css(print_opts));
    } else {
        html.push_str("\n/* Default Print & PDF Stylesheet */\n");
        html.push_str(crate::ui::print_css::DEFAULT_PAGE_CSS);
        html.push_str(crate::ui::print_css::DEFAULT_PRINT_CSS);
    }
    if let Some(custom) = &options.custom_css {
        html.push_str("\n/* Custom user CSS */\n");
        html.push_str(custom);
        html.push('\n');
    }
    html.push_str("  </style>\n");
    html.push_str("</head>\n<body>\n");

    // Main App Shell
    html.push_str("  <div class=\"app-container\">\n");

    // Header & Stats
    if options.include_metadata_header {
        render_header(&mut html, session, &safe_title, &id_str);
    }

    // Action / Filter Toolbar
    render_toolbar(&mut html);

    // Transcript Timeline
    html.push_str("    <main id=\"transcript\" class=\"timeline\">\n");

    let mut message_index = 0usize;
    for msg in &session.messages {
        // Filter out system messages if option disabled
        if msg.role == Role::System && !options.include_system_messages {
            continue;
        }
        // Filter out tool results if option disabled
        if msg.role == Role::Tool && !options.include_tool_calls {
            continue;
        }

        message_index += 1;
        render_message(
            &mut html,
            msg,
            message_index,
            options.include_tool_calls,
            options.syntax_highlighting,
        );
    }

    if message_index == 0 {
        html.push_str("      <div class=\"empty-state\">\n");
        html.push_str("        <div class=\"empty-icon\">💬</div>\n");
        html.push_str("        <h3>No messages in this session transcript</h3>\n");
        html.push_str("        <p>Start a conversation in Fusion to generate content.</p>\n");
        html.push_str("      </div>\n");
    }

    html.push_str("    </main>\n");

    // Footer
    html.push_str("    <footer class=\"app-footer\">\n");
    html.push_str(
        "      <p>Generated by <strong>Fusion</strong> &bull; AI Coding Assistant &bull; ",
    );
    html.push_str(&format!(
        "Exported at <time>{}</time></p>\n",
        escape_html(&chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string())
    ));
    html.push_str("    </footer>\n");

    html.push_str("  </div>\n"); // .app-container

    // Embedded JavaScript
    html.push_str("  <script>\n");
    html.push_str(EMBEDDED_JS);
    html.push_str("  </script>\n");

    html.push_str("</body>\n</html>\n");

    html
}

// -----------------------------------------------------------------------------
// HTML Rendering Helpers
// -----------------------------------------------------------------------------

fn render_header(html: &mut String, session: &Session, title: &str, id_str: &str) {
    html.push_str("    <header class=\"app-header\">\n");
    html.push_str("      <div class=\"header-top\">\n");
    html.push_str("        <div class=\"brand-badge\">\n");
    html.push_str("          <span class=\"brand-icon\">⚡</span>\n");
    html.push_str("          <span class=\"brand-name\">FUSION</span>\n");
    html.push_str("          <span class=\"brand-tag\">TRANSCRIPT</span>\n");
    html.push_str("        </div>\n");
    html.push_str("        <div class=\"header-actions\">\n");
    html.push_str("          <button id=\"theme-toggle\" class=\"btn btn-sm\" title=\"Toggle dark / light theme\" aria-label=\"Toggle theme\">\n");
    html.push_str("            <span class=\"theme-icon-dark\">🌙</span>\n");
    html.push_str("            <span class=\"theme-icon-light\">☀️</span>\n");
    html.push_str("            <span class=\"theme-label\">Theme</span>\n");
    html.push_str("          </button>\n");
    html.push_str("          <button id=\"print-btn\" class=\"btn btn-sm\" onclick=\"window.print()\" title=\"Print or export to PDF\">\n");
    html.push_str("            <span>🖨️</span> Print\n");
    html.push_str("          </button>\n");
    html.push_str("        </div>\n");
    html.push_str("      </div>\n");

    html.push_str("      <h1 class=\"session-title\">");
    html.push_str(title);
    html.push_str("</h1>\n");

    // Metadata chips
    html.push_str("      <div class=\"meta-chips\">\n");

    // Session ID
    html.push_str(&format!(
        "        <div class=\"meta-chip\" title=\"Session UUID\"><span class=\"meta-icon\">🆔</span> <code class=\"session-id\">{}</code></div>\n",
        &id_str[..id_str.len().min(8)]
    ));

    // Active Model
    html.push_str(&format!(
        "        <div class=\"meta-chip meta-chip-model\" title=\"Active Model\"><span class=\"meta-icon\">🧠</span> <strong>{}</strong></div>\n",
        escape_html(&session.active_model)
    ));

    // Created At
    html.push_str(&format!(
        "        <div class=\"meta-chip\" title=\"Created\"><span class=\"meta-icon\">📅</span> {}</div>\n",
        escape_html(&session.created_at)
    ));

    // Working Directory
    if let Some(wd) = &session.working_dir {
        html.push_str(&format!(
            "        <div class=\"meta-chip\" title=\"Working Directory\"><span class=\"meta-icon\">📁</span> <code>{}</code></div>\n",
            escape_html(&wd.display().to_string())
        ));
    }

    html.push_str("      </div>\n");

    // Stats Grid
    let stats = &session.token_stats;
    html.push_str("      <div class=\"stats-grid\">\n");
    html.push_str(&format!(
        "        <div class=\"stat-card\">\n          <span class=\"stat-label\">Total Tokens</span>\n          <span class=\"stat-value\">{}</span>\n        </div>\n",
        format_number(stats.total_tokens)
    ));
    html.push_str(&format!(
        "        <div class=\"stat-card\">\n          <span class=\"stat-label\">Prompt Tokens</span>\n          <span class=\"stat-value text-blue\">{}</span>\n        </div>\n",
        format_number(stats.prompt_tokens)
    ));
    html.push_str(&format!(
        "        <div class=\"stat-card\">\n          <span class=\"stat-label\">Completion Tokens</span>\n          <span class=\"stat-value text-purple\">{}</span>\n        </div>\n",
        format_number(stats.completion_tokens)
    ));
    if stats.cache_read_tokens > 0 || stats.cache_write_tokens > 0 {
        html.push_str(&format!(
            "        <div class=\"stat-card\">\n          <span class=\"stat-label\">Cache (Read/Write)</span>\n          <span class=\"stat-value text-emerald\">{}/{}</span>\n        </div>\n",
            format_number(stats.cache_read_tokens),
            format_number(stats.cache_write_tokens)
        ));
    }
    html.push_str(&format!(
        "        <div class=\"stat-card\">\n          <span class=\"stat-label\">Turns</span>\n          <span class=\"stat-value text-amber\">{}</span>\n        </div>\n",
        stats.total_turns
    ));
    html.push_str("      </div>\n");

    html.push_str("    </header>\n");
}

fn render_toolbar(html: &mut String) {
    html.push_str("    <section class=\"toolbar\" aria-label=\"Transcript Controls\">\n");
    html.push_str("      <div class=\"search-box\">\n");
    html.push_str("        <span class=\"search-icon\">🔍</span>\n");
    html.push_str("        <input type=\"search\" id=\"search-input\" placeholder=\"Search transcript messages...\" autocomplete=\"off\" spellcheck=\"false\">\n");
    html.push_str("        <span id=\"search-count\" class=\"search-count\"></span>\n");
    html.push_str("      </div>\n");
    html.push_str("      <div class=\"filter-group\" role=\"tablist\" aria-label=\"Filter by role\">\n");
    html.push_str("        <button class=\"filter-btn active\" data-role=\"all\">All</button>\n");
    html.push_str("        <button class=\"filter-btn\" data-role=\"user\">👤 User</button>\n");
    html.push_str("        <button class=\"filter-btn\" data-role=\"assistant\">🤖 Assistant</button>\n");
    html.push_str("        <button class=\"filter-btn\" data-role=\"tool\">🔧 Tools</button>\n");
    html.push_str("        <button class=\"filter-btn\" data-role=\"system\">⚙️ System</button>\n");
    html.push_str("      </div>\n");
    html.push_str("      <div class=\"toolbar-actions\">\n");
    html.push_str("        <button id=\"toggle-all-details\" class=\"btn btn-sm btn-ghost\" title=\"Expand or collapse all tool calls and reasoning\">Toggle Details</button>\n");
    html.push_str("      </div>\n");
    html.push_str("    </section>\n");
}

fn render_message(
    html: &mut String,
    msg: &Message,
    index: usize,
    include_tool_calls: bool,
    syntax_highlight: bool,
) {
    let (role_class, role_name, role_icon) = match msg.role {
        Role::System => ("role-system", "System", "⚙️"),
        Role::User => ("role-user", "User", "👤"),
        Role::Assistant => ("role-assistant", "Assistant", "🤖"),
        Role::Tool => ("role-tool", "Tool Output", "🔧"),
    };

    html.push_str(&format!(
        "      <article class=\"message-card {} animate-in\" data-role=\"{}\" id=\"msg-{}\">\n",
        role_class,
        match msg.role {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
        },
        index
    ));

    // Message Card Header
    html.push_str("        <div class=\"message-header\">\n");
    html.push_str("          <div class=\"role-info\">\n");
    html.push_str(&format!("            <span class=\"role-avatar\">{}</span>\n", role_icon));
    html.push_str(&format!(
        "            <span class=\"role-title\">{}</span>\n",
        role_name
    ));
    if let Some(name) = &msg.name {
        html.push_str(&format!(
            "            <span class=\"role-author\">({})</span>\n",
            escape_html(name)
        ));
    }
    if let Some(tool_call_id) = &msg.tool_call_id {
        html.push_str(&format!(
            "            <span class=\"tool-call-badge\" title=\"Tool Call ID\"><code>{}</code></span>\n",
            escape_html(tool_call_id)
        ));
    }
    html.push_str("          </div>\n");

    html.push_str("          <div class=\"message-actions\">\n");
    html.push_str(&format!("            <span class=\"turn-index\">#{}</span>\n", index));
    html.push_str(&format!(
        "            <button class=\"copy-btn\" data-target=\"body-{}\" title=\"Copy message content\" aria-label=\"Copy message content\">📋 Copy</button>\n",
        index
    ));
    html.push_str("          </div>\n");
    html.push_str("        </div>\n");

    // Message Content Body
    html.push_str(&format!(
        "        <div class=\"message-body\" id=\"body-{}\">\n",
        index
    ));

    match msg.role {
        Role::System => {
            // System prompts rendered in a neat collapsible box
            html.push_str("          <details class=\"system-details\" open>\n");
            html.push_str("            <summary class=\"system-summary\">System Instructions</summary>\n");
            html.push_str("            <div class=\"system-content markdown-body\">\n");
            render_markdown_to_html(html, &msg.content, syntax_highlight);
            html.push_str("            </div>\n");
            html.push_str("          </details>\n");
        }
        Role::User => {
            html.push_str("          <div class=\"markdown-body\">\n");
            render_markdown_to_html(html, &msg.content, syntax_highlight);
            html.push_str("          </div>\n");
        }
        Role::Assistant => {
            // Check for DeepSeek / reasoning `<think>...</think>` tags
            let (thinking, remaining_content) = extract_thinking_block(&msg.content);

            if let Some(thought) = thinking {
                html.push_str("          <details class=\"thinking-box\" open>\n");
                html.push_str("            <summary class=\"thinking-header\">\n");
                html.push_str("              <span class=\"thinking-icon\">🧠</span>\n");
                html.push_str("              <span class=\"thinking-title\">Reasoning Process</span>\n");
                html.push_str("            </summary>\n");
                html.push_str("            <div class=\"thinking-content markdown-body\">\n");
                render_markdown_to_html(html, &thought, syntax_highlight);
                html.push_str("            </div>\n");
                html.push_str("          </details>\n");
            }

            if !remaining_content.trim().is_empty() {
                html.push_str("          <div class=\"markdown-body\">\n");
                render_markdown_to_html(html, &remaining_content, syntax_highlight);
                html.push_str("          </div>\n");
            }

            // Render Tool Calls attached to assistant message
            if include_tool_calls {
                if let Some(tool_calls) = &msg.tool_calls {
                    for tool_call in tool_calls {
                        render_tool_call(html, tool_call, syntax_highlight);
                    }
                }
            }
        }
        Role::Tool => {
            // Render tool output as terminal/console block
            html.push_str("          <div class=\"tool-output-container\">\n");
            html.push_str("            <div class=\"tool-output-header\">\n");
            html.push_str("              <span class=\"terminal-dot dot-red\"></span>\n");
            html.push_str("              <span class=\"terminal-dot dot-yellow\"></span>\n");
            html.push_str("              <span class=\"terminal-dot dot-green\"></span>\n");
            html.push_str("              <span class=\"tool-output-title\">Standard Output</span>\n");
            html.push_str("            </div>\n");
            html.push_str("            <pre class=\"tool-output-pre\"><code>");
            html.push_str(&escape_html(&msg.content));
            html.push_str("</code></pre>\n");
            html.push_str("          </div>\n");
        }
    }

    html.push_str("        </div>\n"); // .message-body
    html.push_str("      </article>\n");
}

fn render_tool_call(html: &mut String, tool_call: &ToolCall, syntax_highlight: bool) {
    html.push_str("          <div class=\"tool-call-card\">\n");
    html.push_str("            <div class=\"tool-call-header\">\n");
    html.push_str("              <span class=\"tool-badge\">🛠️ Tool Call</span>\n");
    html.push_str(&format!(
        "              <span class=\"tool-name\"><code>{}</code></span>\n",
        escape_html(&tool_call.name)
    ));
    html.push_str(&format!(
        "              <span class=\"tool-id\">({})</span>\n",
        escape_html(&tool_call.id)
    ));
    html.push_str("            </div>\n");
    html.push_str("            <div class=\"tool-call-args\">\n");

    let formatted_args = format_json_pretty(&tool_call.arguments);
    if syntax_highlight {
        let highlighted = highlight_code(&formatted_args, "json");
        html.push_str(&format!(
            "              <pre class=\"code-block json\"><code>{}</code></pre>\n",
            highlighted
        ));
    } else {
        html.push_str(&format!(
            "              <pre class=\"code-block json\"><code>{}</code></pre>\n",
            escape_html(&formatted_args)
        ));
    }

    html.push_str("            </div>\n");
    html.push_str("          </div>\n");
}

// -----------------------------------------------------------------------------
// Markdown to HTML Conversion
// -----------------------------------------------------------------------------

/// Renders markdown text into clean HTML, handling headers, code blocks, lists,
/// blockquotes, tables, and inline styles.
pub fn render_markdown_to_html(output: &mut String, markdown: &str, syntax_highlight: bool) {
    let mut in_code_block = false;
    let mut code_lang = String::new();
    let mut code_buffer = String::new();
    let mut in_list = false;
    let mut list_is_ordered = false;
    let mut in_table = false;
    let mut table_rows: Vec<Vec<String>> = Vec::new();
    let mut paragraph_buffer = String::new();

    let flush_paragraph = |output: &mut String, buffer: &mut String| {
        if !buffer.trim().is_empty() {
            output.push_str("<p>");
            output.push_str(&render_inline_markdown(buffer.trim()));
            output.push_str("</p>\n");
            buffer.clear();
        }
    };

    let flush_list = |output: &mut String, in_list: &mut bool, ordered: bool| {
        if *in_list {
            if ordered {
                output.push_str("</ol>\n");
            } else {
                output.push_str("</ul>\n");
            }
            *in_list = false;
        }
    };

    let flush_table = |output: &mut String,
                       in_table: &mut bool,
                       rows: &mut Vec<Vec<String>>| {
        if *in_table && !rows.is_empty() {
            output.push_str("<div class=\"table-wrapper\"><table>\n");
            // First row is header
            if let Some(header) = rows.first() {
                output.push_str("<thead><tr>");
                for cell in header {
                    output.push_str("<th>");
                    output.push_str(&render_inline_markdown(cell.trim()));
                    output.push_str("</th>");
                }
                output.push_str("</tr></thead>\n");
            }
            if rows.len() > 1 {
                output.push_str("<tbody>\n");
                for row in rows.iter().skip(1) {
                    output.push_str("<tr>");
                    for cell in row {
                        output.push_str("<td>");
                        output.push_str(&render_inline_markdown(cell.trim()));
                        output.push_str("</td>");
                    }
                    output.push_str("</tr>\n");
                }
                output.push_str("</tbody>\n");
            }
            output.push_str("</table></div>\n");
            rows.clear();
            *in_table = false;
        }
    };

    for line in markdown.lines() {
        let trimmed = line.trim();

        // 1. Code block fence
        if trimmed.starts_with("```") {
            if in_code_block {
                // Closing fence
                flush_paragraph(output, &mut paragraph_buffer);
                output.push_str("<div class=\"code-block-container\">\n");
                output.push_str("  <div class=\"code-block-header\">\n");
                output.push_str(&format!(
                    "    <span class=\"code-lang\">{}</span>\n",
                    if code_lang.is_empty() {
                        "text"
                    } else {
                        &code_lang
                    }
                ));
                output.push_str("    <button class=\"copy-code-btn\" title=\"Copy code block\">Copy</button>\n");
                output.push_str("  </div>\n");

                let formatted_code = if syntax_highlight {
                    highlight_code(&code_buffer, &code_lang)
                } else {
                    escape_html(&code_buffer)
                };

                output.push_str(&format!(
                    "  <pre class=\"code-block {}\"><code>{}</code></pre>\n",
                    escape_html(&code_lang),
                    formatted_code
                ));
                output.push_str("</div>\n");

                code_buffer.clear();
                code_lang.clear();
                in_code_block = false;
            } else {
                // Opening fence
                flush_paragraph(output, &mut paragraph_buffer);
                flush_list(output, &mut in_list, list_is_ordered);
                flush_table(output, &mut in_table, &mut table_rows);
                in_code_block = true;
                code_lang = trimmed.trim_start_matches('`').trim().to_string();
            }
            continue;
        }

        if in_code_block {
            code_buffer.push_str(line);
            code_buffer.push('\n');
            continue;
        }

        // 2. Table row: starts and ends with `|`
        if trimmed.starts_with('|') && trimmed.ends_with('|') && trimmed.len() > 2 {
            flush_paragraph(output, &mut paragraph_buffer);
            flush_list(output, &mut in_list, list_is_ordered);

            // Check if separator line (e.g. |---|---|)
            let is_separator = trimmed
                .split('|')
                .filter(|s| !s.trim().is_empty())
                .all(|s| s.trim().chars().all(|c| c == '-' || c == ':' || c == ' '));

            if !is_separator {
                let cells: Vec<String> = trimmed
                    .split('|')
                    .map(|s| s.to_string())
                    .collect();
                // strip leading/trailing empty cells from split
                let inner_cells = if cells.len() >= 2 {
                    cells[1..cells.len() - 1].to_vec()
                } else {
                    cells
                };
                in_table = true;
                table_rows.push(inner_cells);
            }
            continue;
        } else if in_table {
            flush_table(output, &mut in_table, &mut table_rows);
        }

        // 3. Blank line
        if trimmed.is_empty() {
            flush_paragraph(output, &mut paragraph_buffer);
            flush_list(output, &mut in_list, list_is_ordered);
            continue;
        }

        // 4. Headings (# h1 through ###### h6)
        if let Some(heading_content) = trimmed.strip_prefix("# ") {
            flush_paragraph(output, &mut paragraph_buffer);
            flush_list(output, &mut in_list, list_is_ordered);
            output.push_str(&format!(
                "<h1>{}</h1>\n",
                render_inline_markdown(heading_content)
            ));
            continue;
        } else if let Some(heading_content) = trimmed.strip_prefix("## ") {
            flush_paragraph(output, &mut paragraph_buffer);
            flush_list(output, &mut in_list, list_is_ordered);
            output.push_str(&format!(
                "<h2>{}</h2>\n",
                render_inline_markdown(heading_content)
            ));
            continue;
        } else if let Some(heading_content) = trimmed.strip_prefix("### ") {
            flush_paragraph(output, &mut paragraph_buffer);
            flush_list(output, &mut in_list, list_is_ordered);
            output.push_str(&format!(
                "<h3>{}</h3>\n",
                render_inline_markdown(heading_content)
            ));
            continue;
        } else if let Some(heading_content) = trimmed.strip_prefix("#### ") {
            flush_paragraph(output, &mut paragraph_buffer);
            flush_list(output, &mut in_list, list_is_ordered);
            output.push_str(&format!(
                "<h4>{}</h4>\n",
                render_inline_markdown(heading_content)
            ));
            continue;
        } else if let Some(heading_content) = trimmed.strip_prefix("##### ") {
            flush_paragraph(output, &mut paragraph_buffer);
            flush_list(output, &mut in_list, list_is_ordered);
            output.push_str(&format!(
                "<h5>{}</h5>\n",
                render_inline_markdown(heading_content)
            ));
            continue;
        } else if let Some(heading_content) = trimmed.strip_prefix("###### ") {
            flush_paragraph(output, &mut paragraph_buffer);
            flush_list(output, &mut in_list, list_is_ordered);
            output.push_str(&format!(
                "<h6>{}</h6>\n",
                render_inline_markdown(heading_content)
            ));
            continue;
        }

        // 5. Blockquote (> quote)
        if let Some(quote_content) = trimmed.strip_prefix("> ") {
            flush_paragraph(output, &mut paragraph_buffer);
            flush_list(output, &mut in_list, list_is_ordered);
            output.push_str(&format!(
                "<blockquote><p>{}</p></blockquote>\n",
                render_inline_markdown(quote_content)
            ));
            continue;
        }

        // 6. Horizontal Rule (---, ***, ___ >= 3)
        if (trimmed.starts_with("---") || trimmed.starts_with("***") || trimmed.starts_with("___"))
            && trimmed.chars().all(|c| c == '-' || c == '*' || c == '_')
        {
            flush_paragraph(output, &mut paragraph_buffer);
            flush_list(output, &mut in_list, list_is_ordered);
            output.push_str("<hr>\n");
            continue;
        }

        // 7. Unordered Lists (- item, * item, + item)
        if trimmed.starts_with("- ") || trimmed.starts_with("* ") || trimmed.starts_with("+ ") {
            flush_paragraph(output, &mut paragraph_buffer);
            let item_content = &trimmed[2..];

            if !in_list || list_is_ordered {
                flush_list(output, &mut in_list, list_is_ordered);
                output.push_str("<ul>\n");
                in_list = true;
                list_is_ordered = false;
            }

            // Task list checkbox support: - [ ] or - [x]
            if let Some(task_rest) = item_content.strip_prefix("[ ] ") {
                output.push_str(&format!(
                    "  <li class=\"task-item\"><input type=\"checkbox\" disabled> {}</li>\n",
                    render_inline_markdown(task_rest)
                ));
            } else if let Some(task_rest) = item_content.strip_prefix("[x] ").or_else(|| item_content.strip_prefix("[X] ")) {
                output.push_str(&format!(
                    "  <li class=\"task-item\"><input type=\"checkbox\" checked disabled> {}</li>\n",
                    render_inline_markdown(task_rest)
                ));
            } else {
                output.push_str(&format!(
                    "  <li>{}</li>\n",
                    render_inline_markdown(item_content)
                ));
            }
            continue;
        }

        // 8. Ordered Lists (1. item)
        let is_ordered_item = trimmed
            .find('.')
            .map(|dot_pos| {
                let (digits, rest) = trimmed.split_at(dot_pos);
                !digits.is_empty()
                    && digits.chars().all(|c| c.is_ascii_digit())
                    && rest.starts_with(". ")
            })
            .unwrap_or(false);

        if is_ordered_item {
            flush_paragraph(output, &mut paragraph_buffer);
            let dot_pos = trimmed.find('.').unwrap();
            let item_content = trimmed[dot_pos + 2..].trim();

            if !in_list || !list_is_ordered {
                flush_list(output, &mut in_list, list_is_ordered);
                output.push_str("<ol>\n");
                in_list = true;
                list_is_ordered = true;
            }

            output.push_str(&format!(
                "  <li>{}</li>\n",
                render_inline_markdown(item_content)
            ));
            continue;
        }

        // Fallback: regular paragraph line
        if !paragraph_buffer.is_empty() {
            paragraph_buffer.push(' ');
        }
        paragraph_buffer.push_str(trimmed);
    }

    // Flush any remaining buffers
    if in_code_block {
        let formatted = if syntax_highlight {
            highlight_code(&code_buffer, &code_lang)
        } else {
            escape_html(&code_buffer)
        };
        output.push_str(&format!(
            "<div class=\"code-block-container\"><pre class=\"code-block {}\"><code>{}</code></pre></div>\n",
            escape_html(&code_lang),
            formatted
        ));
    }
    flush_paragraph(output, &mut paragraph_buffer);
    flush_list(output, &mut in_list, list_is_ordered);
    flush_table(output, &mut in_table, &mut table_rows);
}

/// Helper to render inline markdown: bold, italic, inline code, links, strikethrough.
pub fn render_inline_markdown(input: &str) -> String {
    let mut out = String::with_capacity(input.len() * 2);
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            // Inline code: `code`
            '`' => {
                let mut code_inner = String::new();
                let mut closed = false;
                while let Some(&next_c) = chars.peek() {
                    chars.next();
                    if next_c == '`' {
                        closed = true;
                        break;
                    }
                    code_inner.push(next_c);
                }
                if closed {
                    out.push_str("<code class=\"inline-code\">");
                    out.push_str(&escape_html(&code_inner));
                    out.push_str("</code>");
                } else {
                    out.push('`');
                    out.push_str(&escape_html(&code_inner));
                }
            }
            // Bold or Italic with `*`
            '*' => {
                if chars.peek() == Some(&'*') {
                    chars.next();
                    // Bold: **bold**
                    let mut bold_inner = String::new();
                    let mut closed = false;
                    while let Some(next_c) = chars.next() {
                        if next_c == '*' && chars.peek() == Some(&'*') {
                            chars.next();
                            closed = true;
                            break;
                        }
                        bold_inner.push(next_c);
                    }
                    if closed {
                        out.push_str("<strong>");
                        out.push_str(&render_inline_markdown(&bold_inner));
                        out.push_str("</strong>");
                    } else {
                        out.push_str("**");
                        out.push_str(&escape_html(&bold_inner));
                    }
                } else {
                    // Italic: *italic*
                    let mut italic_inner = String::new();
                    let mut closed = false;
                    while let Some(next_c) = chars.next() {
                        if next_c == '*' {
                            closed = true;
                            break;
                        }
                        italic_inner.push(next_c);
                    }
                    if closed {
                        out.push_str("<em>");
                        out.push_str(&render_inline_markdown(&italic_inner));
                        out.push_str("</em>");
                    } else {
                        out.push('*');
                        out.push_str(&escape_html(&italic_inner));
                    }
                }
            }
            // Strikethrough: ~~del~~
            '~' => {
                if chars.peek() == Some(&'~') {
                    chars.next();
                    let mut strike_inner = String::new();
                    let mut closed = false;
                    while let Some(next_c) = chars.next() {
                        if next_c == '~' && chars.peek() == Some(&'~') {
                            chars.next();
                            closed = true;
                            break;
                        }
                        strike_inner.push(next_c);
                    }
                    if closed {
                        out.push_str("<del>");
                        out.push_str(&render_inline_markdown(&strike_inner));
                        out.push_str("</del>");
                    } else {
                        out.push_str("~~");
                        out.push_str(&escape_html(&strike_inner));
                    }
                } else {
                    out.push('~');
                }
            }
            // Links: [text](url)
            '[' => {
                let mut text = String::new();
                let mut closed_bracket = false;
                while let Some(next_c) = chars.next() {
                    if next_c == ']' {
                        closed_bracket = true;
                        break;
                    }
                    text.push(next_c);
                }

                if closed_bracket && chars.peek() == Some(&'(') {
                    chars.next(); // Consume '('
                    let mut url = String::new();
                    let mut closed_paren = false;
                    while let Some(next_c) = chars.next() {
                        if next_c == ')' {
                            closed_paren = true;
                            break;
                        }
                        url.push(next_c);
                    }

                    if closed_paren {
                        let clean_url = escape_html(url.trim());
                        // Basic sanity check to prevent javascript: links
                        let safe_url = if clean_url.starts_with("http://")
                            || clean_url.starts_with("https://")
                            || clean_url.starts_with('#')
                            || clean_url.starts_with('/')
                            || clean_url.starts_with("mailto:")
                        {
                            clean_url
                        } else {
                            format!("#{}", clean_url)
                        };

                        out.push_str("<a href=\"");
                        out.push_str(&safe_url);
                        out.push_str("\" target=\"_blank\" rel=\"noopener noreferrer\">");
                        out.push_str(&render_inline_markdown(&text));
                        out.push_str("</a>");
                    } else {
                        out.push('[');
                        out.push_str(&escape_html(&text));
                        out.push_str("](");
                        out.push_str(&escape_html(&url));
                    }
                } else {
                    out.push('[');
                    out.push_str(&render_inline_markdown(&text));
                    if closed_bracket {
                        out.push(']');
                    }
                }
            }
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }

    out
}

/// Extracts `<think>...</think>` blocks commonly emitted by reasoning models (e.g. DeepSeek R1).
pub fn extract_thinking_block(content: &str) -> (Option<String>, String) {
    if let Some(start_idx) = content.find("<think>") {
        let after_start = &content[start_idx + 7..];
        if let Some(end_idx) = after_start.find("</think>") {
            let thinking = after_start[..end_idx].trim().to_string();
            let mut remaining = String::new();
            remaining.push_str(&content[..start_idx]);
            remaining.push_str(&after_start[end_idx + 8..]);
            return (Some(thinking), remaining.trim().to_string());
        }
    }
    (None, content.to_string())
}

// -----------------------------------------------------------------------------
// Pure Rust Syntax Highlighter
// -----------------------------------------------------------------------------

/// Performs syntax highlighting on a code snippet for the specified language.
/// Emits semantic `<span>` tags with classes for dark mode CSS styling.
pub fn highlight_code(code: &str, lang: &str) -> String {
    let normalized_lang = lang.to_lowercase();
    let lang_str = normalized_lang.as_str();

    match lang_str {
        "rs" | "rust" => highlight_tokens(code, TokenizerKind::Rust),
        "py" | "python" => highlight_tokens(code, TokenizerKind::Python),
        "js" | "javascript" | "jsx" | "ts" | "typescript" | "tsx" => {
            highlight_tokens(code, TokenizerKind::JavaScript)
        }
        "json" => highlight_tokens(code, TokenizerKind::Json),
        "sh" | "bash" | "zsh" | "shell" => highlight_tokens(code, TokenizerKind::Bash),
        "go" | "golang" => highlight_tokens(code, TokenizerKind::Go),
        "c" | "cpp" | "cxx" | "h" | "hpp" => highlight_tokens(code, TokenizerKind::C),
        "sql" => highlight_tokens(code, TokenizerKind::Sql),
        "html" | "xml" | "svg" => highlight_markup(code),
        _ => escape_html(code),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TokenizerKind {
    Rust,
    Python,
    JavaScript,
    Json,
    Bash,
    Go,
    C,
    Sql,
}

fn is_keyword(word: &str, kind: TokenizerKind) -> bool {
    match kind {
        TokenizerKind::Rust => matches!(
            word,
            "as" | "async"
                | "await"
                | "break"
                | "const"
                | "continue"
                | "crate"
                | "dyn"
                | "else"
                | "enum"
                | "extern"
                | "false"
                | "fn"
                | "for"
                | "if"
                | "impl"
                | "in"
                | "let"
                | "loop"
                | "match"
                | "mod"
                | "move"
                | "mut"
                | "pub"
                | "ref"
                | "return"
                | "self"
                | "Self"
                | "static"
                | "struct"
                | "super"
                | "trait"
                | "true"
                | "type"
                | "unsafe"
                | "use"
                | "where"
                | "while"
        ),
        TokenizerKind::Python => matches!(
            word,
            "and" | "as"
                | "assert"
                | "async"
                | "await"
                | "break"
                | "class"
                | "continue"
                | "def"
                | "del"
                | "elif"
                | "else"
                | "except"
                | "False"
                | "finally"
                | "for"
                | "from"
                | "global"
                | "if"
                | "import"
                | "in"
                | "is"
                | "lambda"
                | "None"
                | "nonlocal"
                | "not"
                | "or"
                | "pass"
                | "raise"
                | "return"
                | "True"
                | "try"
                | "while"
                | "with"
                | "yield"
        ),
        TokenizerKind::JavaScript => matches!(
            word,
            "async"
                | "await"
                | "break"
                | "case"
                | "catch"
                | "class"
                | "const"
                | "continue"
                | "debugger"
                | "default"
                | "delete"
                | "do"
                | "else"
                | "export"
                | "extends"
                | "false"
                | "finally"
                | "for"
                | "from"
                | "function"
                | "if"
                | "import"
                | "in"
                | "instanceof"
                | "interface"
                | "let"
                | "new"
                | "null"
                | "of"
                | "return"
                | "super"
                | "switch"
                | "this"
                | "throw"
                | "true"
                | "try"
                | "type"
                | "typeof"
                | "undefined"
                | "var"
                | "void"
                | "while"
                | "with"
                | "yield"
        ),
        TokenizerKind::Json => matches!(word, "true" | "false" | "null"),
        TokenizerKind::Bash => matches!(
            word,
            "case"
                | "do"
                | "done"
                | "elif"
                | "else"
                | "esac"
                | "exit"
                | "export"
                | "fi"
                | "for"
                | "function"
                | "if"
                | "in"
                | "local"
                | "return"
                | "select"
                | "then"
                | "time"
                | "until"
                | "while"
                | "echo"
                | "source"
        ),
        TokenizerKind::Go => matches!(
            word,
            "break"
                | "case"
                | "chan"
                | "const"
                | "continue"
                | "default"
                | "defer"
                | "else"
                | "fallthrough"
                | "for"
                | "func"
                | "go"
                | "goto"
                | "if"
                | "import"
                | "interface"
                | "map"
                | "package"
                | "range"
                | "return"
                | "select"
                | "struct"
                | "switch"
                | "type"
                | "var"
                | "true"
                | "false"
                | "nil"
                | "iota"
        ),
        TokenizerKind::C => matches!(
            word,
            "auto"
                | "break"
                | "case"
                | "const"
                | "continue"
                | "default"
                | "do"
                | "else"
                | "enum"
                | "extern"
                | "for"
                | "goto"
                | "if"
                | "register"
                | "return"
                | "signed"
                | "sizeof"
                | "static"
                | "struct"
                | "switch"
                | "typedef"
                | "union"
                | "unsigned"
                | "void"
                | "volatile"
                | "while"
                | "class"
                | "public"
                | "private"
                | "protected"
                | "template"
                | "typename"
                | "namespace"
                | "using"
                | "virtual"
                | "bool"
                | "true"
                | "false"
                | "nullptr"
        ),
        TokenizerKind::Sql => {
            let u = word.to_ascii_uppercase();
            matches!(
                u.as_str(),
                "SELECT"
                    | "FROM"
                    | "WHERE"
                    | "INSERT"
                    | "INTO"
                    | "UPDATE"
                    | "DELETE"
                    | "JOIN"
                    | "LEFT"
                    | "RIGHT"
                    | "INNER"
                    | "OUTER"
                    | "ON"
                    | "GROUP"
                    | "BY"
                    | "ORDER"
                    | "HAVING"
                    | "LIMIT"
                    | "OFFSET"
                    | "AS"
                    | "AND"
                    | "OR"
                    | "NOT"
                    | "NULL"
                    | "CREATE"
                    | "TABLE"
                    | "DROP"
                    | "ALTER"
                    | "INDEX"
                    | "PRIMARY"
                    | "KEY"
                    | "FOREIGN"
                    | "REFERENCES"
                    | "VALUES"
                    | "SET"
                    | "UNION"
                    | "ALL"
                    | "DISTINCT"
                    | "TRUE"
                    | "FALSE"
            )
        }
    }
}

fn is_builtin_type(word: &str, kind: TokenizerKind) -> bool {
    match kind {
        TokenizerKind::Rust => matches!(
            word,
            "u8" | "u16"
                | "u32"
                | "u64"
                | "u128"
                | "usize"
                | "i8"
                | "i16"
                | "i32"
                | "i64"
                | "i128"
                | "isize"
                | "f32"
                | "f64"
                | "bool"
                | "char"
                | "str"
                | "String"
                | "Vec"
                | "Option"
                | "Some"
                | "None"
                | "Result"
                | "Ok"
                | "Err"
                | "Box"
                | "Rc"
                | "Arc"
                | "Path"
                | "PathBuf"
        ),
        TokenizerKind::Go => matches!(
            word,
            "int" | "int8"
                | "int16"
                | "int32"
                | "int64"
                | "uint"
                | "uint8"
                | "uint16"
                | "uint32"
                | "uint64"
                | "uintptr"
                | "float32"
                | "float64"
                | "complex64"
                | "complex128"
                | "byte"
                | "rune"
                | "string"
                | "bool"
                | "error"
        ),
        TokenizerKind::C => matches!(
            word,
            "int" | "char"
                | "float"
                | "double"
                | "short"
                | "long"
                | "int8_t"
                | "int16_t"
                | "int32_t"
                | "int64_t"
                | "uint8_t"
                | "uint16_t"
                | "uint32_t"
                | "uint64_t"
                | "size_t"
                | "string"
                | "vector"
                | "map"
        ),
        _ => false,
    }
}

fn highlight_tokens(code: &str, kind: TokenizerKind) -> String {
    let mut out = String::with_capacity(code.len() * 2);
    let chars: Vec<char> = code.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        let c = chars[i];

        // 1. Comments
        // Line comment: `//` (Rust, JS, Go, C)
        if (kind == TokenizerKind::Rust
            || kind == TokenizerKind::JavaScript
            || kind == TokenizerKind::Go
            || kind == TokenizerKind::C)
            && c == '/'
            && i + 1 < len
            && chars[i + 1] == '/'
        {
            out.push_str("<span class=\"tok-comment\">");
            while i < len && chars[i] != '\n' {
                push_escaped_char(&mut out, chars[i]);
                i += 1;
            }
            out.push_str("</span>");
            continue;
        }

        // Line comment: `#` (Python, Bash)
        if (kind == TokenizerKind::Python || kind == TokenizerKind::Bash) && c == '#' {
            out.push_str("<span class=\"tok-comment\">");
            while i < len && chars[i] != '\n' {
                push_escaped_char(&mut out, chars[i]);
                i += 1;
            }
            out.push_str("</span>");
            continue;
        }

        // Line comment: `--` (SQL)
        if kind == TokenizerKind::Sql && c == '-' && i + 1 < len && chars[i + 1] == '-' {
            out.push_str("<span class=\"tok-comment\">");
            while i < len && chars[i] != '\n' {
                push_escaped_char(&mut out, chars[i]);
                i += 1;
            }
            out.push_str("</span>");
            continue;
        }

        // Block comment: `/* ... */`
        if (kind == TokenizerKind::Rust
            || kind == TokenizerKind::JavaScript
            || kind == TokenizerKind::Go
            || kind == TokenizerKind::C
            || kind == TokenizerKind::Sql)
            && c == '/'
            && i + 1 < len
            && chars[i + 1] == '*'
        {
            out.push_str("<span class=\"tok-comment\">/*");
            i += 2;
            while i < len {
                if chars[i] == '*' && i + 1 < len && chars[i + 1] == '/' {
                    out.push_str("*/</span>");
                    i += 2;
                    break;
                }
                push_escaped_char(&mut out, chars[i]);
                i += 1;
            }
            continue;
        }

        // 2. Strings
        if c == '"' || c == '\'' || (c == '`' && kind == TokenizerKind::JavaScript) {
            let quote = c;
            out.push_str("<span class=\"tok-string\">");
            push_escaped_char(&mut out, quote);
            i += 1;
            while i < len {
                let sc = chars[i];
                push_escaped_char(&mut out, sc);
                i += 1;
                if sc == '\\' && i < len {
                    push_escaped_char(&mut out, chars[i]);
                    i += 1;
                } else if sc == quote {
                    break;
                }
            }
            out.push_str("</span>");
            continue;
        }

        // 3. Numbers
        if c.is_ascii_digit()
            || (c == '.' && i + 1 < len && chars[i + 1].is_ascii_digit())
        {
            let mut num_str = String::new();
            while i < len
                && (chars[i].is_ascii_alphanumeric()
                    || chars[i] == '.'
                    || chars[i] == '_'
                    || chars[i] == 'x'
                    || chars[i] == 'X')
            {
                num_str.push(chars[i]);
                i += 1;
            }
            out.push_str("<span class=\"tok-number\">");
            out.push_str(&escape_html(&num_str));
            out.push_str("</span>");
            continue;
        }

        // 4. Words (Identifiers, Keywords, Types, Functions)
        if c.is_alphabetic() || c == '_' || (kind == TokenizerKind::Bash && c == '$') {
            let mut word = String::new();
            while i < len && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '$') {
                word.push(chars[i]);
                i += 1;
            }

            // Lookahead: is it followed by `(` (Function call)?
            let mut peek_idx = i;
            while peek_idx < len && chars[peek_idx].is_whitespace() {
                peek_idx += 1;
            }
            let is_fn_call = peek_idx < len && chars[peek_idx] == '(';

            if is_keyword(&word, kind) {
                out.push_str("<span class=\"tok-keyword\">");
                out.push_str(&escape_html(&word));
                out.push_str("</span>");
            } else if is_builtin_type(&word, kind)
                || (word.chars().next().map(|ch| ch.is_uppercase()).unwrap_or(false)
                    && kind != TokenizerKind::Sql)
            {
                out.push_str("<span class=\"tok-type\">");
                out.push_str(&escape_html(&word));
                out.push_str("</span>");
            } else if is_fn_call {
                out.push_str("<span class=\"tok-function\">");
                out.push_str(&escape_html(&word));
                out.push_str("</span>");
            } else if word.starts_with('$') {
                out.push_str("<span class=\"tok-variable\">");
                out.push_str(&escape_html(&word));
                out.push_str("</span>");
            } else {
                out.push_str(&escape_html(&word));
            }
            continue;
        }

        // 5. Operators & Punctuation
        match c {
            '&' | '|' | '=' | '+' | '-' | '*' | '/' | '<' | '>' | '!' | '%' | '^' | ':' => {
                out.push_str("<span class=\"tok-operator\">");
                push_escaped_char(&mut out, c);
                out.push_str("</span>");
                i += 1;
            }
            _ => {
                push_escaped_char(&mut out, c);
                i += 1;
            }
        }
    }

    out
}

fn highlight_markup(code: &str) -> String {
    // Simple HTML/XML highlighting for tags and attributes
    let mut out = String::with_capacity(code.len() * 2);
    let chars: Vec<char> = code.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        let c = chars[i];
        if c == '<' {
            if i + 3 < len && &chars[i..i + 4] == ['<', '!', '-', '-'] {
                // HTML comment <!-- ... -->
                out.push_str("<span class=\"tok-comment\">&lt;!--");
                i += 4;
                while i < len {
                    if i + 2 < len && &chars[i..i + 3] == ['-', '-', '>'] {
                        out.push_str("--&gt;</span>");
                        i += 3;
                        break;
                    }
                    push_escaped_char(&mut out, chars[i]);
                    i += 1;
                }
                continue;
            }

            out.push_str("<span class=\"tok-tag\">&lt;");
            i += 1;
            let mut in_tag = true;
            while i < len && in_tag {
                let tc = chars[i];
                if tc == '>' {
                    out.push_str("&gt;</span>");
                    in_tag = false;
                    i += 1;
                } else if tc == '"' || tc == '\'' {
                    // Attribute string
                    out.push_str("<span class=\"tok-string\">");
                    push_escaped_char(&mut out, tc);
                    let q = tc;
                    i += 1;
                    while i < len && chars[i] != q {
                        push_escaped_char(&mut out, chars[i]);
                        i += 1;
                    }
                    if i < len {
                        push_escaped_char(&mut out, chars[i]);
                        i += 1;
                    }
                    out.push_str("</span>");
                } else {
                    push_escaped_char(&mut out, tc);
                    i += 1;
                }
            }
        } else {
            push_escaped_char(&mut out, c);
            i += 1;
        }
    }

    out
}

#[inline]
fn push_escaped_char(out: &mut String, c: char) {
    match c {
        '&' => out.push_str("&amp;"),
        '<' => out.push_str("&lt;"),
        '>' => out.push_str("&gt;"),
        '"' => out.push_str("&quot;"),
        '\'' => out.push_str("&#39;"),
        _ => out.push(c),
    }
}

/// Escapes standard HTML special characters in text strings.
pub fn escape_html(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        push_escaped_char(&mut out, c);
    }
    out
}

/// Formats a raw JSON string with 2-space indentation if valid, or returns as-is.
fn format_json_pretty(json_str: &str) -> String {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(json_str) {
        serde_json::to_string_pretty(&value).unwrap_or_else(|_| json_str.to_string())
    } else {
        json_str.to_string()
    }
}

/// Formats an integer with comma separators (e.g. 14250 -> "14,250").
fn format_number(num: u64) -> String {
    let s = num.to_string();
    let mut result = String::new();
    let chars: Vec<char> = s.chars().collect();
    let len = chars.len();

    for (i, &ch) in chars.iter().enumerate() {
        result.push(ch);
        let rem = len - 1 - i;
        if rem > 0 && rem % 3 == 0 {
            result.push(',');
        }
    }

    result
}

// -----------------------------------------------------------------------------
// Embedded CSS (Self-contained, Dark/Light mode, Responsive, Print)
// -----------------------------------------------------------------------------

const EMBEDDED_CSS: &str = r#"
:root {
  --font-sans: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif;
  --font-mono: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, "Liberation Mono", "Courier New", monospace;
  
  /* Dark Theme (Default) */
  --bg-app: #090d16;
  --bg-header: #0f172a;
  --bg-card: #1e293b;
  --bg-card-hover: #24334a;
  --bg-card-user: #13233e;
  --bg-code: #0b1120;
  --bg-subtle: #1e293b;
  --border-app: #334155;
  --border-card: #334155;
  --border-user: #2563eb;
  --border-subtle: #1e293b;
  
  --text-main: #f8fafc;
  --text-muted: #94a3b8;
  --text-dim: #64748b;
  
  --accent-blue: #38bdf8;
  --accent-purple: #c084fc;
  --accent-emerald: #34d399;
  --accent-amber: #fbbf24;
  --accent-red: #f87171;
  
  /* Syntax Highlighting Tokens (Dark Mode) */
  --tok-kw: #f472b6;
  --tok-str: #a7f3d0;
  --tok-num: #fde047;
  --tok-com: #64748b;
  --tok-fn: #60a5fa;
  --tok-type: #fbbf24;
  --tok-op: #e879f9;
  --tok-tag: #38bdf8;
  --tok-var: #c084fc;
}

[data-theme="light"] {
  --bg-app: #f8fafc;
  --bg-header: #ffffff;
  --bg-card: #ffffff;
  --bg-card-hover: #f1f5f9;
  --bg-card-user: #eff6ff;
  --bg-code: #0f172a;
  --bg-subtle: #e2e8f0;
  --border-app: #cbd5e1;
  --border-card: #e2e8f0;
  --border-user: #93c5fd;
  --border-subtle: #f1f5f9;
  
  --text-main: #0f172a;
  --text-muted: #475569;
  --text-dim: #94a3b8;
  
  --accent-blue: #0284c7;
  --accent-purple: #9333ea;
  --accent-emerald: #059669;
  --accent-amber: #d97706;
  --accent-red: #dc2626;
  
  --tok-kw: #ec4899;
  --tok-str: #059669;
  --tok-num: #d97706;
  --tok-com: #94a3b8;
  --tok-fn: #2563eb;
  --tok-type: #d97706;
  --tok-op: #9333ea;
  --tok-tag: #0284c7;
  --tok-var: #7c3aed;
}

* {
  box-sizing: border-box;
  margin: 0;
  padding: 0;
}

body {
  font-family: var(--font-sans);
  background-color: var(--bg-app);
  color: var(--text-main);
  line-height: 1.6;
  min-height: 100vh;
  padding: 1.5rem 1rem 3rem 1rem;
  transition: background-color 0.2s ease, color 0.2s ease;
}

.app-container {
  max-width: 1000px;
  margin: 0 auto;
}

/* Header & Meta */
.app-header {
  background: var(--bg-header);
  border: 1px solid var(--border-app);
  border-radius: 12px;
  padding: 1.75rem;
  margin-bottom: 1.25rem;
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.15);
}

.header-top {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 1rem;
}

.brand-badge {
  display: inline-flex;
  align-items: center;
  gap: 0.5rem;
  font-weight: 700;
  font-size: 0.9rem;
  letter-spacing: 0.05em;
}

.brand-icon {
  font-size: 1.25rem;
  filter: drop-shadow(0 0 8px rgba(56, 189, 248, 0.5));
}

.brand-name {
  color: var(--accent-blue);
}

.brand-tag {
  background: rgba(56, 189, 248, 0.12);
  color: var(--accent-blue);
  border: 1px solid rgba(56, 189, 248, 0.3);
  font-size: 0.7rem;
  padding: 0.15rem 0.45rem;
  border-radius: 4px;
}

.header-actions {
  display: flex;
  gap: 0.5rem;
}

.session-title {
  font-size: 1.65rem;
  font-weight: 800;
  line-height: 1.3;
  margin-bottom: 0.75rem;
  color: var(--text-main);
}

.meta-chips {
  display: flex;
  flex-wrap: wrap;
  gap: 0.5rem;
  margin-bottom: 1.25rem;
}

.meta-chip {
  display: inline-flex;
  align-items: center;
  gap: 0.35rem;
  background: var(--bg-subtle);
  border: 1px solid var(--border-subtle);
  padding: 0.25rem 0.65rem;
  border-radius: 6px;
  font-size: 0.82rem;
  color: var(--text-muted);
}

.meta-chip-model {
  background: rgba(192, 132, 252, 0.1);
  border-color: rgba(192, 132, 252, 0.25);
  color: var(--accent-purple);
}

.session-id {
  font-family: var(--font-mono);
  font-size: 0.8rem;
  color: var(--accent-blue);
}

.stats-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(130px, 1fr));
  gap: 0.75rem;
}

.stat-card {
  background: var(--bg-subtle);
  border: 1px solid var(--border-subtle);
  border-radius: 8px;
  padding: 0.75rem 0.9rem;
  display: flex;
  flex-direction: column;
  gap: 0.2rem;
}

.stat-label {
  font-size: 0.75rem;
  text-transform: uppercase;
  letter-spacing: 0.04em;
  color: var(--text-dim);
  font-weight: 600;
}

.stat-value {
  font-size: 1.25rem;
  font-weight: 700;
  font-family: var(--font-mono);
  color: var(--text-main);
}

.text-blue { color: var(--accent-blue); }
.text-purple { color: var(--accent-purple); }
.text-emerald { color: var(--accent-emerald); }
.text-amber { color: var(--accent-amber); }

/* Toolbar & Filters */
.toolbar {
  background: var(--bg-header);
  border: 1px solid var(--border-app);
  border-radius: 10px;
  padding: 0.75rem 1rem;
  margin-bottom: 1.5rem;
  display: flex;
  flex-wrap: wrap;
  gap: 0.75rem;
  align-items: center;
  justify-content: space-between;
}

.search-box {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  background: var(--bg-subtle);
  border: 1px solid var(--border-subtle);
  padding: 0.35rem 0.75rem;
  border-radius: 6px;
  flex: 1 1 240px;
  position: relative;
}

.search-box input {
  background: transparent;
  border: none;
  outline: none;
  color: var(--text-main);
  font-family: var(--font-sans);
  font-size: 0.875rem;
  width: 100%;
}

.search-box input::placeholder {
  color: var(--text-dim);
}

.search-count {
  font-size: 0.75rem;
  color: var(--accent-blue);
  font-family: var(--font-mono);
}

.filter-group {
  display: flex;
  gap: 0.35rem;
  flex-wrap: wrap;
}

.filter-btn {
  background: transparent;
  border: 1px solid transparent;
  color: var(--text-muted);
  padding: 0.3rem 0.65rem;
  border-radius: 6px;
  font-size: 0.82rem;
  cursor: pointer;
  font-weight: 500;
  transition: all 0.15s ease;
}

.filter-btn:hover {
  background: var(--bg-subtle);
  color: var(--text-main);
}

.filter-btn.active {
  background: var(--accent-blue);
  color: #0f172a;
  font-weight: 700;
}

.btn {
  background: var(--bg-subtle);
  border: 1px solid var(--border-subtle);
  color: var(--text-main);
  padding: 0.4rem 0.8rem;
  border-radius: 6px;
  font-size: 0.82rem;
  font-weight: 500;
  cursor: pointer;
  display: inline-flex;
  align-items: center;
  gap: 0.35rem;
  transition: all 0.15s ease;
}

.btn:hover {
  background: var(--bg-card-hover);
  border-color: var(--border-app);
}

.btn-ghost {
  background: transparent;
  border-color: transparent;
}

.theme-icon-light { display: none; }
[data-theme="light"] .theme-icon-dark { display: none; }
[data-theme="light"] .theme-icon-light { display: inline; }

/* Timeline & Message Cards */
.timeline {
  display: flex;
  flex-direction: column;
  gap: 1.25rem;
}

.message-card {
  background: var(--bg-card);
  border: 1px solid var(--border-card);
  border-radius: 10px;
  overflow: hidden;
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.08);
  transition: transform 0.15s ease, box-shadow 0.15s ease;
}

.message-card:hover {
  box-shadow: 0 4px 14px rgba(0, 0, 0, 0.12);
}

.message-card.role-user {
  background: var(--bg-card-user);
  border-left: 4px solid var(--accent-blue);
}

.message-card.role-assistant {
  border-left: 4px solid var(--accent-purple);
}

.message-card.role-system {
  border-left: 4px solid var(--accent-amber);
}

.message-card.role-tool {
  border-left: 4px solid var(--accent-emerald);
}

.message-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 0.75rem 1.15rem;
  background: rgba(0, 0, 0, 0.08);
  border-bottom: 1px solid var(--border-card);
}

.role-info {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  font-size: 0.9rem;
}

.role-title {
  font-weight: 700;
  color: var(--text-main);
}

.role-author {
  color: var(--text-muted);
  font-size: 0.8rem;
}

.tool-call-badge code {
  background: var(--bg-subtle);
  border: 1px solid var(--border-subtle);
  font-family: var(--font-mono);
  font-size: 0.75rem;
  padding: 0.15rem 0.4rem;
  border-radius: 4px;
  color: var(--accent-emerald);
}

.message-actions {
  display: flex;
  align-items: center;
  gap: 0.65rem;
}

.turn-index {
  font-family: var(--font-mono);
  font-size: 0.75rem;
  color: var(--text-dim);
}

.copy-btn {
  background: transparent;
  border: 1px solid transparent;
  color: var(--text-dim);
  font-size: 0.75rem;
  padding: 0.2rem 0.5rem;
  border-radius: 4px;
  cursor: pointer;
  transition: all 0.15s ease;
}

.copy-btn:hover {
  background: var(--bg-subtle);
  color: var(--text-main);
}

.message-body {
  padding: 1.25rem 1.4rem;
  font-size: 0.95rem;
  word-wrap: break-word;
}

/* Markdown Typography */
.markdown-body {
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
}

.markdown-body h1, .markdown-body h2, .markdown-body h3, .markdown-body h4 {
  color: var(--text-main);
  font-weight: 700;
  margin-top: 0.5rem;
  line-height: 1.3;
}

.markdown-body h1 { font-size: 1.4rem; border-bottom: 1px solid var(--border-subtle); padding-bottom: 0.3rem; }
.markdown-body h2 { font-size: 1.25rem; }
.markdown-body h3 { font-size: 1.1rem; }
.markdown-body h4 { font-size: 0.98rem; }

.markdown-body p {
  line-height: 1.65;
}

.markdown-body blockquote {
  border-left: 3px solid var(--accent-blue);
  padding: 0.4rem 0 0.4rem 0.9rem;
  color: var(--text-muted);
  background: rgba(56, 189, 248, 0.04);
  border-radius: 0 6px 6px 0;
}

.markdown-body ul, .markdown-body ol {
  padding-left: 1.5rem;
  display: flex;
  flex-direction: column;
  gap: 0.3rem;
}

.markdown-body li {
  line-height: 1.55;
}

.task-item {
  list-style: none;
  margin-left: -1.2rem;
  display: flex;
  align-items: center;
  gap: 0.45rem;
}

.markdown-body hr {
  border: 0;
  border-top: 1px solid var(--border-subtle);
  margin: 0.75rem 0;
}

.markdown-body a {
  color: var(--accent-blue);
  text-decoration: none;
  font-weight: 500;
}

.markdown-body a:hover {
  text-decoration: underline;
}

.inline-code {
  font-family: var(--font-mono);
  background: var(--bg-subtle);
  border: 1px solid var(--border-subtle);
  font-size: 0.85em;
  padding: 0.15rem 0.35rem;
  border-radius: 4px;
  color: var(--accent-amber);
}

/* Tables */
.table-wrapper {
  overflow-x: auto;
  margin: 0.5rem 0;
  border: 1px solid var(--border-subtle);
  border-radius: 6px;
}

table {
  width: 100%;
  border-collapse: collapse;
  text-align: left;
  font-size: 0.88rem;
}

th, td {
  padding: 0.55rem 0.85rem;
  border-bottom: 1px solid var(--border-subtle);
}

th {
  background: var(--bg-subtle);
  font-weight: 700;
  color: var(--text-main);
}

tr:hover td {
  background: rgba(255, 255, 255, 0.02);
}

/* Code Blocks */
.code-block-container {
  margin: 0.65rem 0;
  border-radius: 8px;
  overflow: hidden;
  border: 1px solid var(--border-subtle);
  background: var(--bg-code);
}

.code-block-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 0.35rem 0.85rem;
  background: rgba(255, 255, 255, 0.04);
  border-bottom: 1px solid rgba(255, 255, 255, 0.06);
}

.code-lang {
  font-family: var(--font-mono);
  font-size: 0.75rem;
  font-weight: 600;
  text-transform: uppercase;
  color: var(--text-dim);
}

.copy-code-btn {
  background: transparent;
  border: none;
  color: var(--text-dim);
  font-size: 0.72rem;
  cursor: pointer;
  padding: 0.15rem 0.4rem;
  border-radius: 4px;
  transition: all 0.15s ease;
}

.copy-code-btn:hover {
  background: rgba(255, 255, 255, 0.1);
  color: var(--text-main);
}

.code-block {
  padding: 0.85rem 1rem;
  overflow-x: auto;
  font-family: var(--font-mono);
  font-size: 0.85rem;
  line-height: 1.5;
  color: #f1f5f9;
}

/* Syntax Token Colors */
.tok-keyword { color: var(--tok-kw); font-weight: 600; }
.tok-string { color: var(--tok-str); }
.tok-number { color: var(--tok-num); }
.tok-comment { color: var(--tok-com); font-style: italic; }
.tok-function { color: var(--tok-fn); }
.tok-type { color: var(--tok-type); }
.tok-operator { color: var(--tok-op); }
.tok-tag { color: var(--tok-tag); font-weight: 600; }
.tok-variable { color: var(--tok-var); }

/* Reasoning / Thinking Box (<think>) */
.thinking-box {
  background: rgba(192, 132, 252, 0.05);
  border: 1px solid rgba(192, 132, 252, 0.2);
  border-radius: 8px;
  margin-bottom: 1rem;
  overflow: hidden;
}

.thinking-header {
  padding: 0.6rem 0.9rem;
  font-weight: 600;
  font-size: 0.85rem;
  color: var(--accent-purple);
  cursor: pointer;
  display: flex;
  align-items: center;
  gap: 0.45rem;
  background: rgba(192, 132, 252, 0.08);
}

.thinking-content {
  padding: 0.9rem 1.1rem;
  font-size: 0.9rem;
  color: var(--text-muted);
  border-top: 1px solid rgba(192, 132, 252, 0.15);
}

/* Tool Calls & Outputs */
.tool-call-card {
  background: rgba(251, 191, 36, 0.05);
  border: 1px solid rgba(251, 191, 36, 0.2);
  border-radius: 8px;
  margin-top: 0.75rem;
  overflow: hidden;
}

.tool-call-header {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.5rem 0.85rem;
  background: rgba(251, 191, 36, 0.08);
  font-size: 0.82rem;
}

.tool-badge {
  font-weight: 700;
  color: var(--accent-amber);
}

.tool-name code {
  font-family: var(--font-mono);
  color: var(--text-main);
}

.tool-id {
  color: var(--text-dim);
  font-size: 0.75rem;
}

.tool-call-args pre {
  margin: 0;
  border: none;
  border-radius: 0;
  background: var(--bg-code);
}

.tool-output-container {
  background: var(--bg-code);
  border: 1px solid var(--border-subtle);
  border-radius: 8px;
  overflow: hidden;
}

.tool-output-header {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  padding: 0.45rem 0.85rem;
  background: rgba(255, 255, 255, 0.03);
  border-bottom: 1px solid rgba(255, 255, 255, 0.06);
}

.terminal-dot {
  width: 9px;
  height: 9px;
  border-radius: 50%;
}

.dot-red { background: #ef4444; }
.dot-yellow { background: #f59e0b; }
.dot-green { background: #10b981; }

.tool-output-title {
  font-size: 0.75rem;
  font-family: var(--font-mono);
  color: var(--text-dim);
  margin-left: 0.35rem;
}

.tool-output-pre {
  padding: 0.85rem 1rem;
  font-family: var(--font-mono);
  font-size: 0.85rem;
  line-height: 1.45;
  color: #e2e8f0;
  overflow-x: auto;
  max-height: 480px;
}

/* System Prompt Details */
.system-details {
  background: rgba(100, 116, 139, 0.06);
  border: 1px solid var(--border-subtle);
  border-radius: 8px;
  overflow: hidden;
}

.system-summary {
  padding: 0.55rem 0.85rem;
  font-weight: 600;
  font-size: 0.85rem;
  color: var(--text-muted);
  cursor: pointer;
}

.system-content {
  padding: 0.85rem 1.1rem;
  border-top: 1px solid var(--border-subtle);
}

/* Empty State */
.empty-state {
  text-align: center;
  padding: 4rem 1rem;
  background: var(--bg-card);
  border: 1px dashed var(--border-app);
  border-radius: 12px;
}

.empty-icon {
  font-size: 3rem;
  margin-bottom: 0.5rem;
}

/* Footer */
.app-footer {
  text-align: center;
  margin-top: 2.5rem;
  font-size: 0.8rem;
  color: var(--text-dim);
}


/* Mobile Responsiveness */
@media (max-width: 640px) {
  body {
    padding: 0.75rem 0.5rem;
  }
  
  .app-header {
    padding: 1.15rem 1rem;
  }
  
  .session-title {
    font-size: 1.3rem;
  }
  
  .toolbar {
    flex-direction: column;
    align-items: stretch;
  }
  
  .filter-group {
    justify-content: flex-start;
  }
  
  .message-body {
    padding: 0.9rem 1rem;
  }
}
"#;

// -----------------------------------------------------------------------------
// Embedded Client-side JavaScript (Self-contained, Fast, No dependencies)
// -----------------------------------------------------------------------------

const EMBEDDED_JS: &str = r#"
(function() {
  // 1. Theme Toggle
  const themeToggle = document.getElementById('theme-toggle');
  const htmlRoot = document.documentElement;

  function getActiveTheme() {
    return htmlRoot.getAttribute('data-theme') || 'dark';
  }

  function setTheme(newTheme) {
    htmlRoot.setAttribute('data-theme', newTheme);
    try {
      localStorage.setItem('fusion_export_theme', newTheme);
    } catch (_) {}
  }

  // Restore stored theme if present
  try {
    const saved = localStorage.getItem('fusion_export_theme');
    if (saved === 'light' || saved === 'dark') {
      htmlRoot.setAttribute('data-theme', saved);
    }
  } catch (_) {}

  if (themeToggle) {
    themeToggle.addEventListener('click', () => {
      const current = getActiveTheme();
      const next = current === 'dark' ? 'light' : 'dark';
      setTheme(next);
    });
  }

  // 2. Search & Filtering
  const searchInput = document.getElementById('search-input');
  const searchCount = document.getElementById('search-count');
  const filterBtns = document.querySelectorAll('.filter-btn');
  const messageCards = document.querySelectorAll('.message-card');

  let activeRoleFilter = 'all';
  let searchQuery = '';

  function applyFilters() {
    let visibleCount = 0;
    const query = searchQuery.toLowerCase().trim();

    messageCards.forEach(card => {
      const role = card.getAttribute('data-role');
      const text = card.textContent.toLowerCase();

      const matchesRole = (activeRoleFilter === 'all' || role === activeRoleFilter);
      const matchesSearch = (!query || text.includes(query));

      if (matchesRole && matchesSearch) {
        card.style.display = '';
        visibleCount++;
      } else {
        card.style.display = 'none';
      }
    });

    if (searchCount) {
      if (query || activeRoleFilter !== 'all') {
        searchCount.textContent = `${visibleCount}/${messageCards.length}`;
      } else {
        searchCount.textContent = '';
      }
    }
  }

  if (searchInput) {
    searchInput.addEventListener('input', (e) => {
      searchQuery = e.target.value;
      applyFilters();
    });
  }

  filterBtns.forEach(btn => {
    btn.addEventListener('click', () => {
      filterBtns.forEach(b => b.classList.remove('active'));
      btn.classList.add('active');
      activeRoleFilter = btn.getAttribute('data-role') || 'all';
      applyFilters();
    });
  });

  // 3. Toggle Details (Tool Calls & Reasoning)
  const toggleDetailsBtn = document.getElementById('toggle-all-details');
  let detailsExpanded = true;

  if (toggleDetailsBtn) {
    toggleDetailsBtn.addEventListener('click', () => {
      detailsExpanded = !detailsExpanded;
      const allDetails = document.querySelectorAll('details');
      allDetails.forEach(d => {
        d.open = detailsExpanded;
      });
      toggleDetailsBtn.textContent = detailsExpanded ? 'Collapse Details' : 'Expand Details';
    });
  }

  // 4. Clipboard Copy Handlers
  document.addEventListener('click', (e) => {
    // Copy Message Content Button
    const copyMsgBtn = e.target.closest('.copy-btn');
    if (copyMsgBtn) {
      const targetId = copyMsgBtn.getAttribute('data-target');
      const targetEl = document.getElementById(targetId);
      if (targetEl) {
        navigator.clipboard.writeText(targetEl.innerText).then(() => {
          const original = copyMsgBtn.textContent;
          copyMsgBtn.textContent = '✓ Copied!';
          setTimeout(() => { copyMsgBtn.textContent = original; }, 1800);
        }).catch(() => {
          copyMsgBtn.textContent = 'Failed';
        });
      }
      return;
    }

    // Copy Code Block Button
    const copyCodeBtn = e.target.closest('.copy-code-btn');
    if (copyCodeBtn) {
      const container = copyCodeBtn.closest('.code-block-container');
      const codeEl = container ? container.querySelector('code') : null;
      if (codeEl) {
        navigator.clipboard.writeText(codeEl.innerText).then(() => {
          const original = copyCodeBtn.textContent;
          copyCodeBtn.textContent = '✓ Copied!';
          setTimeout(() => { copyCodeBtn.textContent = original; }, 1800);
        }).catch(() => {
          copyCodeBtn.textContent = 'Failed';
        });
      }
    }
  });
})();
"#;

// -----------------------------------------------------------------------------
// Unit Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::session::TokenStats;
    use uuid::Uuid;

    fn make_test_session() -> Session {
        let mut session = Session {
            id: Uuid::new_v4(),
            created_at: "2026-09-02T10:00:00Z".to_string(),
            updated_at: "2026-09-02T10:15:00Z".to_string(),
            active_model: "anthropic/claude-3.5-sonnet".to_string(),
            title: Some("Rust Architecture Discussion".to_string()),
            messages: Vec::new(),
            token_stats: TokenStats::new(),
            system_prompt: Some("You are a helpful coding assistant.".to_string()),
            working_dir: None,
            metadata: std::collections::HashMap::new(),
        };

        session.token_stats.add(150, 220);

        session.messages.push(Message::system("You are an expert systems programmer."));
        session.messages.push(Message::user("How do I structure a fast Rust CLI?"));
        session.messages.push(Message::assistant_with_tools(
            "<think>Analyzing architecture options for Rust CLI apps.</think>Here is a breakdown using **clap** and **tokio**:\n\n```rust\nfn main() {\n    println!(\"Hello World\");\n}\n```",
            vec![ToolCall {
                id: "call_123".to_string(),
                name: "file_search".to_string(),
                arguments: r#"{"query":"src/main.rs"}"#.to_string(),
            }],
        ));
        session.messages.push(Message::tool_result("call_123", "Found 1 matching file."));

        session
    }

    #[test]
    fn test_export_session_html_basic() {
        let session = make_test_session();
        let html = export_session_html(&session);

        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("Rust Architecture Discussion"));
        assert!(html.contains("anthropic/claude-3.5-sonnet"));
        assert!(html.contains("How do I structure a fast Rust CLI?"));
        assert!(html.contains("Reasoning Process"));
        assert!(html.contains("tok-keyword")); // Syntax highlighting
        assert!(html.contains("file_search"));
        assert!(html.contains("call_123"));
        assert!(html.contains("Found 1 matching file."));
        assert!(html.contains("data-theme=\"dark\""));
    }

    #[test]
    fn test_export_options_customization() {
        let session = make_test_session();
        let options = ExportOptions::new()
            .with_title("Custom Export Title")
            .with_theme(ExportTheme::Light)
            .with_system_messages(false)
            .with_tool_calls(false)
            .with_custom_css(".custom-class { color: red; }");

        let html = export_session_html_with_options(&session, &options);

        assert!(html.contains("Custom Export Title"));
        assert!(html.contains("data-theme=\"light\""));
        assert!(!html.contains("You are an expert systems programmer.")); // Filtered out
        assert!(!html.contains("file_search")); // Tool calls filtered out
        assert!(html.contains(".custom-class { color: red; }"));
    }

    #[test]
    fn test_html_escaping_xss_protection() {
        let mut session = make_test_session();
        session.messages.push(Message::user("<script>alert('xss')</script> & dangerous < >"));

        let html = export_session_html(&session);
        assert!(!html.contains("<script>alert('xss')</script>"));
        assert!(html.contains("&lt;script&gt;alert(&#39;xss&#39;)&lt;/script&gt;"));
    }

    #[test]
    fn test_markdown_rendering() {
        let mut out = String::new();
        let md = "# Heading 1\n## Heading 2\n\nParagraph with **bold** and *italic* and `code`.\n\n- [ ] Item 1\n- [x] Item 2\n\n> A quote\n\n| H1 | H2 |\n|---|---|\n| C1 | C2 |";
        render_markdown_to_html(&mut out, md, true);

        assert!(out.contains("<h1>Heading 1</h1>"));
        assert!(out.contains("<h2>Heading 2</h2>"));
        assert!(out.contains("<strong>bold</strong>"));
        assert!(out.contains("<em>italic</em>"));
        assert!(out.contains("<code class=\"inline-code\">code</code>"));
        assert!(out.contains("task-item"));
        assert!(out.contains("<blockquote>"));
        assert!(out.contains("<table"));
        assert!(out.contains("<th>H1</th>"));
        assert!(out.contains("<td>C1</td>"));
    }

    #[test]
    fn test_syntax_highlighter_languages() {
        // Rust
        let rs = highlight_code("fn main() -> Result<(), ()> { let x = 42; }", "rust");
        assert!(rs.contains("tok-keyword"));
        assert!(rs.contains("tok-number"));

        // Python
        let py = highlight_code("def calculate(a, b):\n    return a + b # sum", "python");
        assert!(py.contains("tok-keyword"));
        assert!(py.contains("tok-comment"));

        // JSON
        let json = highlight_code(r#"{"key": "value", "count": 10, "active": true}"#, "json");
        assert!(json.contains("tok-string"));
        assert!(json.contains("tok-number"));
        assert!(json.contains("tok-keyword"));

        // Bash
        let sh = highlight_code("export PATH=\"/usr/local/bin:$PATH\"\necho $PATH", "bash");
        assert!(sh.contains("tok-keyword"));
        assert!(sh.contains("tok-string"));
    }

    #[test]
    fn test_extract_thinking_block() {
        let raw = "<think>Analyzing code</think>Final answer here.";
        let (think, remaining) = extract_thinking_block(raw);
        assert_eq!(think.as_deref(), Some("Analyzing code"));
        assert_eq!(remaining, "Final answer here.");

        let no_think = "Just a direct answer.";
        let (none_think, rem) = extract_thinking_block(no_think);
        assert!(none_think.is_none());
        assert_eq!(rem, "Just a direct answer.");
    }
}
