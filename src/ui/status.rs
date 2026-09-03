//! Compact single-line status bar widget for terminal and inline UI.
//!
//! Renders a high-density, beautifully styled status bar displaying:
//! - **Active model** (e.g. `anthropic/claude-3-7-sonnet`, `deepseek-chat`, `gpt-4o`)
//! - **Git branch** (e.g. `⎇ main`, `⎇ feat/ui`, detected automatically or set explicitly)
//! - **Tokens used** (e.g. `1.2k tokens`, `45.8k tok`, `1.5M`)
//! - **Session duration** (e.g. `45s`, `2m 14s`, `1h 05m`)
//! - **USD cost** (e.g. `$0.00`, `$0.012`, `$1.25`)
//! - Optional subagent, advisor critique badge, and activity status message
//!
//! Supports both Ratatui `Widget` rendering into terminal buffers and standalone
//! ANSI / plain text line generation for mobile/Termux, desktop CLI, and log exports.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Widget, Wrap},
    Frame,
};

use crate::agent::cost::CostBreakdown;
use crate::agent::session::TokenStats;
use crate::ui::inline::StatusInfo;
use crate::ui::theme::Theme;

// ---------------------------------------------------------------------------
// 1. Git Branch Auto-Detection
// ---------------------------------------------------------------------------

/// Detects the current active Git branch for the workspace without spawning subprocesses if possible.
///
/// Fast algorithm:
/// 1. Locates `.git` (directory or worktree pointer file) searching upwards from current working directory.
/// 2. Parses `HEAD` file directly:
///    - If `ref: refs/heads/<branch>`, extracts the branch name.
///    - If detached HEAD (commit SHA), extracts the 7-character short commit hash (e.g. `detached:a1b2c3d`).
/// 3. If `.git` is a worktree file (`gitdir: <path>`), resolves the worktree `HEAD`.
/// 4. Falls back to `git rev-parse --abbrev-ref HEAD` subprocess if direct inspection fails.
pub fn detect_git_branch() -> Option<String> {
    if let Ok(cwd) = std::env::current_dir() {
        detect_git_branch_in(&cwd)
    } else {
        None
    }
}

/// Detects the active Git branch starting from the specified path.
pub fn detect_git_branch_in(start_path: &Path) -> Option<String> {
    // 1. Direct filesystem inspection (zero process overhead)
    if let Some(branch) = detect_git_branch_from_fs(start_path) {
        return Some(branch);
    }

    // 2. Subprocess fallback
    detect_git_branch_from_cmd(start_path)
}

/// Direct filesystem inspection for `.git/HEAD`.
fn detect_git_branch_from_fs(start_path: &Path) -> Option<String> {
    let mut current = if start_path.is_file() {
        start_path.parent()?.to_path_buf()
    } else {
        start_path.to_path_buf()
    };

    let mut git_marker_path: Option<PathBuf> = None;

    loop {
        let candidate = current.join(".git");
        if candidate.exists() {
            git_marker_path = Some(candidate);
            break;
        }
        if !current.pop() {
            break;
        }
    }

    let git_marker = git_marker_path?;

    // Determine the actual git metadata directory
    let git_dir = if git_marker.is_file() {
        // Worktree or submodule: `.git` file contains `gitdir: <path>`
        let content = std::fs::read_to_string(&git_marker).ok()?;
        let trimmed = content.trim();
        let target = if let Some(stripped) = trimmed.strip_prefix("gitdir:") {
            stripped.trim()
        } else {
            return None;
        };
        let resolved = if Path::new(target).is_absolute() {
            PathBuf::from(target)
        } else {
            git_marker.parent()?.join(target)
        };
        resolved
    } else {
        git_marker
    };

    let head_path = git_dir.join("HEAD");
    if !head_path.is_file() {
        return None;
    }

    let head_content = std::fs::read_to_string(head_path).ok()?;
    let head_str = head_content.trim();

    if let Some(ref_path) = head_str.strip_prefix("ref:") {
        let ref_clean = ref_path.trim();
        if let Some(branch_name) = ref_clean.strip_prefix("refs/heads/") {
            return Some(branch_name.to_string());
        }
        // Other ref pointer (e.g. refs/tags/v1.0)
        return Some(ref_clean.to_string());
    }

    // Detached HEAD: 40-hex SHA hash -> truncate to 7 characters
    if head_str.len() >= 7 && head_str.chars().all(|c| c.is_ascii_hexdigit()) {
        return Some(format!("detached:{}", &head_str[..7]));
    }

    None
}

/// Fallback Git branch detection using standard git binary.
fn detect_git_branch_from_cmd(working_dir: &Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(working_dir)
        .output()
        .ok()?;

    if output.status.success() {
        let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !branch.is_empty() && branch != "HEAD" {
            return Some(branch);
        }
    }

    // If HEAD is detached, try getting short commit hash
    let commit_out = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(working_dir)
        .output()
        .ok()?;

    if commit_out.status.success() {
        let hash = String::from_utf8_lossy(&commit_out.stdout)
            .trim()
            .to_string();
        if !hash.is_empty() {
            return Some(format!("detached:{}", hash));
        }
    }

    None
}

// ---------------------------------------------------------------------------
// 2. Compact Formatting Utilities
// ---------------------------------------------------------------------------

/// Formats a token count into a compact human-readable string (e.g. `450`, `1.2k`, `45.8k`, `1.5M`).
pub fn format_token_compact(tokens: u64) -> String {
    if tokens >= 1_000_000 {
        let val = tokens as f64 / 1_000_000.0;
        format!("{:.1}M", val)
    } else if tokens >= 1_000 {
        let val = tokens as f64 / 1_000.0;
        format!("{:.1}k", val)
    } else {
        tokens.to_string()
    }
}

/// Formats a session duration into a compact human-readable string (e.g. `450ms`, `45s`, `2m 14s`, `1h 05m`).
pub fn format_duration_compact(d: Duration) -> String {
    let total_secs = d.as_secs();
    let millis = d.subsec_millis();

    if total_secs == 0 {
        if millis == 0 {
            "0s".to_string()
        } else {
            format!("{}ms", millis)
        }
    } else if total_secs < 60 {
        format!("{}s", total_secs)
    } else if total_secs < 3600 {
        let mins = total_secs / 60;
        let secs = total_secs % 60;
        if secs == 0 {
            format!("{}m", mins)
        } else {
            format!("{}m {:02}s", mins, secs)
        }
    } else {
        let hours = total_secs / 3600;
        let mins = (total_secs % 3600) / 60;
        format!("{}h {:02}m", hours, mins)
    }
}

/// Formats a USD financial cost into a clean, compact string (e.g. `$0.00`, `$0.0042`, `$1.25`).
pub fn format_cost_compact(cost: f64) -> String {
    if cost.abs() < 1e-9 {
        "$0.00".to_string()
    } else if cost < 0.0001 {
        format!("${:.6}", cost)
    } else if cost < 0.01 {
        format!("${:.4}", cost)
    } else if cost < 10.0 {
        format!("${:.3}", cost)
    } else {
        format!("${:.2}", cost)
    }
}

/// Formats model name with optional provider prefix.
pub fn format_model_badge(provider: Option<&str>, model: &str, compact: bool) -> String {
    if compact {
        // Strip long vendor prefixes if redundant for compact mode
        model.to_string()
    } else if let Some(p) = provider {
        if !p.is_empty() && !model.starts_with(&format!("{}/", p)) {
            format!("{}/{}", p, model)
        } else {
            model.to_string()
        }
    } else {
        model.to_string()
    }
}

/// Formats git branch with icon or ASCII prefix.
pub fn format_branch_badge(branch: &str, show_icon: bool) -> String {
    if show_icon {
        format!("⎇ {}", branch)
    } else {
        format!("git:{}", branch)
    }
}

// ---------------------------------------------------------------------------
// 3. Status Bar Display Mode
// ---------------------------------------------------------------------------

/// Display density mode for the status bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StatusBarMode {
    /// Automatically selects density based on available terminal width.
    #[default]
    Auto,
    /// Ultra-compact single line for narrow/mobile terminals (< 50 cols).
    Minimal,
    /// High-density compact single line (50-80 cols).
    Compact,
    /// Standard single line with full badges (80-110 cols).
    Normal,
    /// Wide layout with expanded metrics and status (> 110 cols).
    Full,
}

// ---------------------------------------------------------------------------
// 4. StatusBar Widget Data Structure & Builder
// ---------------------------------------------------------------------------

/// A compact, single-line status bar widget.
///
/// Encapsulates:
/// - Active model name and optional provider
/// - Git branch (with optional auto-detection)
/// - Tokens consumed
/// - Session duration
/// - Accumulated USD cost
/// - Optional subagent, advisor, and status message
#[derive(Debug, Clone, PartialEq)]
pub struct StatusBar {
    /// Active model identifier (e.g. `"claude-3-7-sonnet"`).
    pub model: String,
    /// Optional active provider name (e.g. `"anthropic"`).
    pub provider: Option<String>,
    /// Active git branch name (e.g. `"main"`).
    pub git_branch: Option<String>,
    /// Total tokens consumed in the session.
    pub tokens_used: u64,
    /// Optional prompt tokens count.
    pub prompt_tokens: Option<u64>,
    /// Optional completion tokens count.
    pub completion_tokens: Option<u64>,
    /// Session duration elapsed.
    pub session_duration: Option<Duration>,
    /// Estimated cost in USD.
    pub usd_cost: Option<f64>,
    /// Active status or activity message (e.g. `"Thinking..."`, `"Editing src/main.rs"`).
    pub status_message: Option<String>,
    /// Active subagent badge (e.g. `"Coder"`).
    pub active_agent: Option<String>,
    /// Active advisor critique badge (e.g. `"SecAdvisor"`).
    pub active_advisor: Option<String>,
    /// Optional context window usage percentage (0.0 to 100.0).
    pub context_percentage: Option<f32>,
    /// Display density mode.
    pub mode: StatusBarMode,
    /// Whether to render Unicode glyphs (e.g. `✦`, `⎇`) or plain ASCII.
    pub show_icons: bool,
    /// Whether to render a top border block or raw single line.
    pub show_border: bool,
    /// Theme for styling and colors.
    pub theme: Option<Theme>,
    /// Custom delimiter separator (defaults to `" │ "` or `" · "`).
    pub custom_separator: Option<String>,
}

impl Default for StatusBar {
    fn default() -> Self {
        Self {
            model: "default".to_string(),
            provider: None,
            git_branch: None,
            tokens_used: 0,
            prompt_tokens: None,
            completion_tokens: None,
            session_duration: None,
            usd_cost: None,
            status_message: None,
            active_agent: None,
            active_advisor: None,
            context_percentage: None,
            mode: StatusBarMode::Auto,
            show_icons: true,
            show_border: false,
            theme: None,
            custom_separator: None,
        }
    }
}

impl StatusBar {
    /// Creates a new `StatusBar` with the given active model name.
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            ..Default::default()
        }
    }

    /// Creates a new `StatusBar` with provider and model.
    pub fn with_model_and_provider(provider: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            provider: Some(provider.into()),
            model: model.into(),
            ..Default::default()
        }
    }

    /// Sets the provider name.
    pub fn with_provider(mut self, provider: impl Into<String>) -> Self {
        self.provider = Some(provider.into());
        self
    }

    /// Sets the model name.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    /// Sets the git branch name explicitly.
    pub fn with_git_branch(mut self, branch: impl Into<String>) -> Self {
        self.git_branch = Some(branch.into());
        self
    }

    /// Auto-detects the git branch from the current working directory.
    pub fn with_auto_git_branch(mut self) -> Self {
        self.git_branch = detect_git_branch();
        self
    }

    /// Sets the total tokens used.
    pub fn with_tokens(mut self, tokens: u64) -> Self {
        self.tokens_used = tokens;
        self
    }

    /// Sets token usage statistics from `TokenStats`.
    pub fn with_token_stats(mut self, stats: &TokenStats) -> Self {
        self.tokens_used = stats.total_tokens;
        self.prompt_tokens = Some(stats.prompt_tokens);
        self.completion_tokens = Some(stats.completion_tokens);
        self
    }

    /// Sets the session duration.
    pub fn with_duration(mut self, duration: Duration) -> Self {
        self.session_duration = Some(duration);
        self
    }

    /// Sets the session duration in seconds.
    pub fn with_duration_secs(mut self, secs: u64) -> Self {
        self.session_duration = Some(Duration::from_secs(secs));
        self
    }

    /// Sets the session duration calculated from an initial `Instant`.
    pub fn with_session_start(mut self, start: Instant) -> Self {
        self.session_duration = Some(start.elapsed());
        self
    }

    /// Sets the accumulated USD cost.
    pub fn with_cost(mut self, cost_usd: f64) -> Self {
        self.usd_cost = Some(cost_usd);
        self
    }

    /// Sets the cost from a `CostBreakdown`.
    pub fn with_cost_breakdown(mut self, breakdown: &CostBreakdown) -> Self {
        self.usd_cost = Some(breakdown.total_cost);
        self
    }

    /// Sets the active activity status message.
    pub fn with_status(mut self, status: impl Into<String>) -> Self {
        let s = status.into();
        self.status_message = if s.is_empty() { None } else { Some(s) };
        self
    }

    /// Sets the active subagent badge.
    pub fn with_agent(mut self, agent: impl Into<String>) -> Self {
        let a = agent.into();
        self.active_agent = if a.is_empty() { None } else { Some(a) };
        self
    }

    /// Sets the active advisor badge.
    pub fn with_advisor(mut self, advisor: impl Into<String>) -> Self {
        let a = advisor.into();
        self.active_advisor = if a.is_empty() { None } else { Some(a) };
        self
    }

    /// Sets context window usage percentage (0.0 to 100.0).
    pub fn with_context_percentage(mut self, percentage: f32) -> Self {
        self.context_percentage = Some(percentage.clamp(0.0, 100.0));
        self
    }

    /// Sets the display density mode.
    pub fn with_mode(mut self, mode: StatusBarMode) -> Self {
        self.mode = mode;
        self
    }

    /// Enables or disables Unicode icons.
    pub fn with_icons(mut self, show_icons: bool) -> Self {
        self.show_icons = show_icons;
        self
    }

    /// Enables or disables top border rendering.
    pub fn with_border(mut self, show_border: bool) -> Self {
        self.show_border = show_border;
        self
    }

    /// Sets the active theme.
    pub fn with_theme(mut self, theme: Theme) -> Self {
        self.theme = Some(theme);
        self
    }

    /// Sets a custom separator delimiter between sections.
    pub fn with_separator(mut self, separator: impl Into<String>) -> Self {
        self.custom_separator = Some(separator.into());
        self
    }

    // -----------------------------------------------------------------------
    // Effective Mode & Delimiter Helpers
    // -----------------------------------------------------------------------

    /// Resolves the effective `StatusBarMode` for a given available width.
    pub fn effective_mode(&self, width: u16) -> StatusBarMode {
        match self.mode {
            StatusBarMode::Auto => {
                if width < 48 {
                    StatusBarMode::Minimal
                } else if width < 76 {
                    StatusBarMode::Compact
                } else if width < 110 {
                    StatusBarMode::Normal
                } else {
                    StatusBarMode::Full
                }
            }
            explicit => explicit,
        }
    }

    /// Returns the active separator string.
    pub fn separator(&self, mode: StatusBarMode) -> &str {
        if let Some(custom) = &self.custom_separator {
            custom.as_str()
        } else {
            match mode {
                StatusBarMode::Minimal => " · ",
                StatusBarMode::Compact => " │ ",
                StatusBarMode::Normal => " │ ",
                StatusBarMode::Full => " │ ",
                StatusBarMode::Auto => " │ ",
            }
        }
    }

    // -----------------------------------------------------------------------
    // Span Construction for Ratatui
    // -----------------------------------------------------------------------

    /// Builds styled Ratatui `Span` items for the status bar line given a terminal width.
    pub fn build_spans(&self, width: u16, theme: &Theme) -> Vec<Span<'static>> {
        let eff_mode = self.effective_mode(width);
        let sep_str = self.separator(eff_mode).to_string();
        let sep_style = theme.muted_style();

        // Color definitions
        let model_style = theme.model_style();
        let provider_style = theme.provider_style();
        let git_style = Style::default()
            .fg(Color::Rgb(158, 206, 106))
            .add_modifier(Modifier::BOLD);
        let token_style = Style::default().fg(Color::Rgb(224, 175, 104));
        let duration_style = Style::default().fg(Color::Rgb(122, 162, 247));
        let cost_style = Style::default()
            .fg(Color::Rgb(115, 218, 202))
            .add_modifier(Modifier::BOLD);
        let agent_style = theme.agent_style();
        let advisor_style = theme.advisor_style();
        let status_style = theme.status_style();

        let mut spans = Vec::new();
        let mut first = true;

        let mut push_sep = |spans: &mut Vec<Span<'static>>| {
            if !first {
                spans.push(Span::styled(sep_str.clone(), sep_style));
            }
            first = false;
        };

        // 1. Model & Provider Section
        push_sep(&mut spans);
        if self.show_icons && eff_mode != StatusBarMode::Minimal {
            spans.push(Span::styled("✦ ", provider_style));
        }

        match eff_mode {
            StatusBarMode::Minimal => {
                // Short model name
                let short_model = truncate_str(&self.model, 14).to_string();
                spans.push(Span::styled(short_model, model_style));
            }
            StatusBarMode::Compact => {
                let model_str = format_model_badge(self.provider.as_deref(), &self.model, true);
                let truncated = truncate_str(&model_str, 20).to_string();
                spans.push(Span::styled(truncated, model_style));
            }
            StatusBarMode::Normal | StatusBarMode::Full => {
                if let Some(prov) = &self.provider {
                    if !prov.is_empty() && !self.model.starts_with(&format!("{}/", prov)) {
                        spans.push(Span::styled(prov.clone(), provider_style));
                        spans.push(Span::styled("/".to_string(), sep_style));
                    }
                }
                spans.push(Span::styled(self.model.clone(), model_style));
            }
            StatusBarMode::Auto => unreachable!(),
        }

        // 2. Git Branch Section
        if let Some(branch) = &self.git_branch {
            push_sep(&mut spans);
            let branch_text = match eff_mode {
                StatusBarMode::Minimal => {
                    let short_b = truncate_str(branch, 10);
                    if self.show_icons {
                        format!("⎇{}", short_b)
                    } else {
                        short_b.to_string()
                    }
                }
                StatusBarMode::Compact => {
                    let short_b = truncate_str(branch, 14);
                    format_branch_badge(&short_b, self.show_icons)
                }
                StatusBarMode::Normal | StatusBarMode::Full => {
                    format_branch_badge(branch, self.show_icons)
                }
                StatusBarMode::Auto => unreachable!(),
            };
            spans.push(Span::styled(branch_text, git_style));
        }

        // 3. Tokens Used Section
        push_sep(&mut spans);
        let token_text = match eff_mode {
            StatusBarMode::Minimal => format_token_compact(self.tokens_used),
            StatusBarMode::Compact => format!("{} tok", format_token_compact(self.tokens_used)),
            StatusBarMode::Normal => {
                if let (Some(prompt), Some(comp)) = (self.prompt_tokens, self.completion_tokens) {
                    format!(
                        "{} tokens ({} / {})",
                        format_token_compact(self.tokens_used),
                        format_token_compact(prompt),
                        format_token_compact(comp)
                    )
                } else {
                    format!("{} tokens", format_token_compact(self.tokens_used))
                }
            }
            StatusBarMode::Full => {
                if let (Some(prompt), Some(comp)) = (self.prompt_tokens, self.completion_tokens) {
                    format!(
                        "{} tokens (in: {}, out: {})",
                        format_token_compact(self.tokens_used),
                        format_token_compact(prompt),
                        format_token_compact(comp)
                    )
                } else {
                    format!("{} tokens", format_token_compact(self.tokens_used))
                }
            }
            StatusBarMode::Auto => unreachable!(),
        };
        spans.push(Span::styled(token_text, token_style));

        // 4. Session Duration Section
        if let Some(duration) = self.session_duration {
            push_sep(&mut spans);
            let dur_text = match eff_mode {
                StatusBarMode::Minimal | StatusBarMode::Compact => {
                    format_duration_compact(duration)
                }
                StatusBarMode::Normal | StatusBarMode::Full => {
                    if self.show_icons {
                        format!("⏱ {}", format_duration_compact(duration))
                    } else {
                        format_duration_compact(duration)
                    }
                }
                StatusBarMode::Auto => unreachable!(),
            };
            spans.push(Span::styled(dur_text, duration_style));
        }

        // 5. USD Cost Section
        if let Some(cost) = self.usd_cost {
            push_sep(&mut spans);
            let cost_text = format_cost_compact(cost);
            spans.push(Span::styled(cost_text, cost_style));
        }

        // 6. Optional Badges for Normal / Full density
        if matches!(eff_mode, StatusBarMode::Normal | StatusBarMode::Full) {
            // Context usage percentage
            if let Some(pct) = self.context_percentage {
                push_sep(&mut spans);
                let ctx_text = format!("{:.0}% ctx", pct);
                let ctx_style = if pct > 80.0 {
                    theme.warning_style()
                } else {
                    theme.muted_style()
                };
                spans.push(Span::styled(ctx_text, ctx_style));
            }

            // Subagent badge
            if let Some(agent) = &self.active_agent {
                push_sep(&mut spans);
                let tag = format!("Agent:{}", agent);
                spans.push(Span::styled(tag, agent_style));
            }

            // Advisor badge
            if let Some(advisor) = &self.active_advisor {
                push_sep(&mut spans);
                let tag = format!("Adv:{}", advisor);
                spans.push(Span::styled(tag, advisor_style));
            }

            // Status message
            if let Some(status) = &self.status_message {
                if !status.is_empty() {
                    push_sep(&mut spans);
                    let clean_status = if eff_mode == StatusBarMode::Normal {
                        truncate_str(status, 24).to_string()
                    } else {
                        status.clone()
                    };
                    spans.push(Span::styled(clean_status, status_style));
                }
            }
        }

        spans
    }

    /// Builds a single Ratatui `Line` from the status bar state.
    pub fn to_line(&self, width: u16, theme: &Theme) -> Line<'static> {
        Line::from(self.build_spans(width, theme))
    }

    /// Renders the status bar directly into a Ratatui frame.
    pub fn render_to_frame(&self, f: &mut Frame, area: Rect) {
        let theme = self.theme.clone().unwrap_or_else(Theme::auto);
        let spans = self.build_spans(area.width, &theme);
        let line = Line::from(spans);

        let mut p = Paragraph::new(line).wrap(Wrap { trim: true });

        if self.show_border {
            p = p.block(
                Block::default()
                    .borders(Borders::TOP)
                    .border_type(BorderType::Plain)
                    .border_style(theme.border_style()),
            );
        }

        f.render_widget(p, area);
    }

    /// Renders the status bar directly into a Ratatui `Buffer`.
    pub fn render_buffer(&self, area: Rect, buf: &mut Buffer) {
        let theme = self.theme.clone().unwrap_or_else(Theme::auto);
        let spans = self.build_spans(area.width, &theme);
        let line = Line::from(spans);

        let mut p = Paragraph::new(line).wrap(Wrap { trim: true });

        if self.show_border {
            p = p.block(
                Block::default()
                    .borders(Borders::TOP)
                    .border_type(BorderType::Plain)
                    .border_style(theme.border_style()),
            );
        }

        p.render(area, buf);
    }

    // -----------------------------------------------------------------------
    // Plain Text & ANSI Line Generation
    // -----------------------------------------------------------------------

    /// Renders the status bar as an ANSI colorized single-line string.
    pub fn render_ansi(&self, width: usize, theme: Option<&Theme>) -> String {
        let default_theme = Theme::auto();
        let _th = theme.or(self.theme.as_ref()).unwrap_or(&default_theme);
        let eff_mode = self.effective_mode(width as u16);
        let sep = self.separator(eff_mode);

        let mut parts = Vec::new();

        // 1. Model & Provider
        let model_str = match eff_mode {
            StatusBarMode::Minimal => {
                format!("\x1b[1;36m{}\x1b[0m", truncate_str(&self.model, 14))
            }
            StatusBarMode::Compact => {
                let badge = format_model_badge(self.provider.as_deref(), &self.model, true);
                let icon = if self.show_icons { "✦ " } else { "" };
                format!("\x1b[1;36m{}{}\x1b[0m", icon, truncate_str(&badge, 20))
            }
            StatusBarMode::Normal | StatusBarMode::Full => {
                let icon = if self.show_icons { "✦ " } else { "" };
                if let Some(prov) = &self.provider {
                    if !prov.is_empty() && !self.model.starts_with(&format!("{}/", prov)) {
                        format!(
                            "\x1b[1;35m{}{}\x1b[0m/\x1b[1;36m{}\x1b[0m",
                            icon, prov, self.model
                        )
                    } else {
                        format!("\x1b[1;36m{}{}\x1b[0m", icon, self.model)
                    }
                } else {
                    format!("\x1b[1;36m{}{}\x1b[0m", icon, self.model)
                }
            }
            StatusBarMode::Auto => unreachable!(),
        };
        parts.push(model_str);

        // 2. Git Branch
        if let Some(branch) = &self.git_branch {
            let branch_str = match eff_mode {
                StatusBarMode::Minimal => {
                    let short_b = truncate_str(branch, 10);
                    let prefix = if self.show_icons { "⎇" } else { "" };
                    format!("\x1b[1;32m{}{}\x1b[0m", prefix, short_b)
                }
                StatusBarMode::Compact => {
                    let short_b = truncate_str(branch, 14);
                    format!(
                        "\x1b[1;32m{}\x1b[0m",
                        format_branch_badge(&short_b, self.show_icons)
                    )
                }
                StatusBarMode::Normal | StatusBarMode::Full => {
                    format!(
                        "\x1b[1;32m{}\x1b[0m",
                        format_branch_badge(branch, self.show_icons)
                    )
                }
                StatusBarMode::Auto => unreachable!(),
            };
            parts.push(branch_str);
        }

        // 3. Tokens Used
        let tok_str = match eff_mode {
            StatusBarMode::Minimal => {
                format!("\x1b[33m{}\x1b[0m", format_token_compact(self.tokens_used))
            }
            StatusBarMode::Compact => {
                format!(
                    "\x1b[33m{} tok\x1b[0m",
                    format_token_compact(self.tokens_used)
                )
            }
            StatusBarMode::Normal | StatusBarMode::Full => {
                if let (Some(prompt), Some(comp)) = (self.prompt_tokens, self.completion_tokens) {
                    format!(
                        "\x1b[33m{} tokens ({} / {})\x1b[0m",
                        format_token_compact(self.tokens_used),
                        format_token_compact(prompt),
                        format_token_compact(comp)
                    )
                } else {
                    format!(
                        "\x1b[33m{} tokens\x1b[0m",
                        format_token_compact(self.tokens_used)
                    )
                }
            }
            StatusBarMode::Auto => unreachable!(),
        };
        parts.push(tok_str);

        // 4. Session Duration
        if let Some(dur) = self.session_duration {
            let icon = if self.show_icons
                && matches!(eff_mode, StatusBarMode::Normal | StatusBarMode::Full)
            {
                "⏱ "
            } else {
                ""
            };
            let dur_str = format!("\x1b[34m{}{}\x1b[0m", icon, format_duration_compact(dur));
            parts.push(dur_str);
        }

        // 5. USD Cost
        if let Some(cost) = self.usd_cost {
            let cost_str = format!("\x1b[1;32m{}\x1b[0m", format_cost_compact(cost));
            parts.push(cost_str);
        }

        // 6. Optional badges
        if matches!(eff_mode, StatusBarMode::Normal | StatusBarMode::Full) {
            if let Some(agent) = &self.active_agent {
                parts.push(format!("\x1b[1;34mAgent:{}\x1b[0m", agent));
            }
            if let Some(advisor) = &self.active_advisor {
                parts.push(format!("\x1b[1;33mAdv:{}\x1b[0m", advisor));
            }
            if let Some(status) = &self.status_message {
                if !status.is_empty() {
                    parts.push(format!("\x1b[36m{}\x1b[0m", status));
                }
            }
        }

        let sep_ansi = format!("\x1b[90m{}\x1b[0m", sep);
        parts.join(&sep_ansi)
    }

    /// Renders the status bar as a plain-text single-line string (no ANSI escape codes).
    pub fn render_plain(&self, width: usize) -> String {
        let eff_mode = self.effective_mode(width as u16);
        let sep = self.separator(eff_mode);

        let mut parts = Vec::new();

        // 1. Model & Provider
        let model_str = match eff_mode {
            StatusBarMode::Minimal => truncate_str(&self.model, 14).to_string(),
            StatusBarMode::Compact => {
                let badge = format_model_badge(self.provider.as_deref(), &self.model, true);
                let icon = if self.show_icons { "✦ " } else { "" };
                format!("{}{}", icon, truncate_str(&badge, 20))
            }
            StatusBarMode::Normal | StatusBarMode::Full => {
                let icon = if self.show_icons { "✦ " } else { "" };
                if let Some(prov) = &self.provider {
                    if !prov.is_empty() && !self.model.starts_with(&format!("{}/", prov)) {
                        format!("{}{}/{}", icon, prov, self.model)
                    } else {
                        format!("{}{}", icon, self.model)
                    }
                } else {
                    format!("{}{}", icon, self.model)
                }
            }
            StatusBarMode::Auto => unreachable!(),
        };
        parts.push(model_str);

        // 2. Git Branch
        if let Some(branch) = &self.git_branch {
            let branch_str = match eff_mode {
                StatusBarMode::Minimal => {
                    let short_b = truncate_str(branch, 10);
                    let prefix = if self.show_icons { "⎇" } else { "" };
                    format!("{}{}", prefix, short_b)
                }
                StatusBarMode::Compact => {
                    let short_b = truncate_str(branch, 14);
                    format_branch_badge(&short_b, self.show_icons)
                }
                StatusBarMode::Normal | StatusBarMode::Full => {
                    format_branch_badge(branch, self.show_icons)
                }
                StatusBarMode::Auto => unreachable!(),
            };
            parts.push(branch_str);
        }

        // 3. Tokens Used
        let tok_str = match eff_mode {
            StatusBarMode::Minimal => format_token_compact(self.tokens_used),
            StatusBarMode::Compact => format!("{} tok", format_token_compact(self.tokens_used)),
            StatusBarMode::Normal | StatusBarMode::Full => {
                if let (Some(prompt), Some(comp)) = (self.prompt_tokens, self.completion_tokens) {
                    format!(
                        "{} tokens ({} / {})",
                        format_token_compact(self.tokens_used),
                        format_token_compact(prompt),
                        format_token_compact(comp)
                    )
                } else {
                    format!("{} tokens", format_token_compact(self.tokens_used))
                }
            }
            StatusBarMode::Auto => unreachable!(),
        };
        parts.push(tok_str);

        // 4. Session Duration
        if let Some(dur) = self.session_duration {
            let icon = if self.show_icons
                && matches!(eff_mode, StatusBarMode::Normal | StatusBarMode::Full)
            {
                "⏱ "
            } else {
                ""
            };
            parts.push(format!("{}{}", icon, format_duration_compact(dur)));
        }

        // 5. USD Cost
        if let Some(cost) = self.usd_cost {
            parts.push(format_cost_compact(cost));
        }

        // 6. Optional badges
        if matches!(eff_mode, StatusBarMode::Normal | StatusBarMode::Full) {
            if let Some(agent) = &self.active_agent {
                parts.push(format!("Agent:{}", agent));
            }
            if let Some(advisor) = &self.active_advisor {
                parts.push(format!("Adv:{}", advisor));
            }
            if let Some(status) = &self.status_message {
                if !status.is_empty() {
                    parts.push(status.clone());
                }
            }
        }

        parts.join(sep)
    }
}

// ---------------------------------------------------------------------------
// 5. Ratatui Widget Implementations
// ---------------------------------------------------------------------------

impl Widget for &StatusBar {
    fn render(self, area: Rect, buf: &mut Buffer) {
        self.render_buffer(area, buf);
    }
}

impl Widget for StatusBar {
    fn render(self, area: Rect, buf: &mut Buffer) {
        self.render_buffer(area, buf);
    }
}

// ---------------------------------------------------------------------------
// 6. Interoperability with StatusInfo
// ---------------------------------------------------------------------------

impl From<&StatusInfo> for StatusBar {
    fn from(info: &StatusInfo) -> Self {
        let mut bar = StatusBar::new(&info.model);
        if !info.provider.is_empty() {
            bar.provider = Some(info.provider.clone());
        }
        bar.active_agent = info.active_agent.clone();
        bar.active_advisor = info.active_advisor.clone();
        if !info.status.is_empty() {
            bar.status_message = Some(info.status.clone());
        }
        bar
    }
}

impl From<&StatusBar> for StatusInfo {
    fn from(bar: &StatusBar) -> Self {
        let mut info = StatusInfo::new(bar.provider.clone().unwrap_or_default(), bar.model.clone());
        info.active_agent = bar.active_agent.clone();
        info.active_advisor = bar.active_advisor.clone();
        if let Some(status) = &bar.status_message {
            info.status = status.clone();
        }
        if bar.tokens_used > 0 || bar.usd_cost.is_some() {
            let tok_str = format_token_compact(bar.tokens_used);
            let cost_str = bar.usd_cost.map(format_cost_compact);
            let summary = match cost_str {
                Some(cost) => format!("{} tokens / {}", tok_str, cost),
                None => format!("{} tokens", tok_str),
            };
            info.token_usage = Some(summary);
        }
        info
    }
}

// ---------------------------------------------------------------------------
// 7. Internal Helpers
// ---------------------------------------------------------------------------

/// Safely truncates a UTF-8 string to at most `max_chars` code points with an ellipsis.
fn truncate_str(s: &str, max_chars: usize) -> &str {
    if max_chars == 0 {
        return "";
    }
    let char_count = s.chars().count();
    if char_count <= max_chars {
        s
    } else {
        let byte_idx = s
            .char_indices()
            .nth(max_chars.saturating_sub(1))
            .map(|(idx, _)| idx)
            .unwrap_or(s.len());
        &s[..byte_idx]
    }
}

// ---------------------------------------------------------------------------
// 8. Unit Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::fs::File;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn test_format_token_compact() {
        assert_eq!(format_token_compact(0), "0");
        assert_eq!(format_token_compact(450), "450");
        assert_eq!(format_token_compact(999), "999");
        assert_eq!(format_token_compact(1000), "1.0k");
        assert_eq!(format_token_compact(1250), "1.3k");
        assert_eq!(format_token_compact(45800), "45.8k");
        assert_eq!(format_token_compact(1000000), "1.0M");
        assert_eq!(format_token_compact(1540000), "1.5M");
    }

    #[test]
    fn test_format_duration_compact() {
        assert_eq!(format_duration_compact(Duration::from_millis(0)), "0s");
        assert_eq!(format_duration_compact(Duration::from_millis(350)), "350ms");
        assert_eq!(format_duration_compact(Duration::from_secs(45)), "45s");
        assert_eq!(format_duration_compact(Duration::from_secs(60)), "1m");
        assert_eq!(format_duration_compact(Duration::from_secs(134)), "2m 14s");
        assert_eq!(format_duration_compact(Duration::from_secs(3600)), "1h 00m");
        assert_eq!(format_duration_compact(Duration::from_secs(3915)), "1h 05m");
    }

    #[test]
    fn test_format_cost_compact() {
        assert_eq!(format_cost_compact(0.0), "$0.00");
        assert_eq!(format_cost_compact(0.000015), "$0.000015");
        assert_eq!(format_cost_compact(0.0042), "$0.0042");
        assert_eq!(format_cost_compact(0.012), "$0.012");
        assert_eq!(format_cost_compact(1.25), "$1.250");
        assert_eq!(format_cost_compact(12.5), "$12.50");
    }

    #[test]
    fn test_git_branch_detection_from_fs() {
        let temp = tempdir().unwrap();
        let git_dir = temp.path().join(".git");
        std::fs::create_dir(&git_dir).unwrap();

        // Standard branch ref
        let head_path = git_dir.join("HEAD");
        let mut f = File::create(&head_path).unwrap();
        writeln!(f, "ref: refs/heads/feature/status-widget").unwrap();

        let branch = detect_git_branch_in(temp.path()).unwrap();
        assert_eq!(branch, "feature/status-widget");

        // Detached HEAD
        let mut f = File::create(&head_path).unwrap();
        writeln!(f, "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2").unwrap();

        let branch_detached = detect_git_branch_in(temp.path()).unwrap();
        assert_eq!(branch_detached, "detached:a1b2c3d");
    }

    #[test]
    fn test_git_branch_worktree_detection() {
        let temp = tempdir().unwrap();
        let actual_git_dir = temp.path().join("main_repo").join(".git");
        let worktree_git_dir = actual_git_dir.join("worktrees").join("wt1");
        std::fs::create_dir_all(&worktree_git_dir).unwrap();

        let head_path = worktree_git_dir.join("HEAD");
        let mut f = File::create(&head_path).unwrap();
        writeln!(f, "ref: refs/heads/fix-worktree").unwrap();

        let worktree_dir = temp.path().join("wt1_workdir");
        std::fs::create_dir_all(&worktree_dir).unwrap();
        let git_file = worktree_dir.join(".git");
        let mut gf = File::create(&git_file).unwrap();
        writeln!(gf, "gitdir: {}", worktree_git_dir.display()).unwrap();

        let branch = detect_git_branch_in(&worktree_dir).unwrap();
        assert_eq!(branch, "fix-worktree");
    }

    #[test]
    fn test_status_bar_builder_and_plain_rendering() {
        let bar = StatusBar::new("claude-3-7-sonnet")
            .with_provider("anthropic")
            .with_git_branch("main")
            .with_tokens(14200)
            .with_duration(Duration::from_secs(134))
            .with_cost(0.0125)
            .with_status("Ready");

        let plain = bar.render_plain(120);
        assert!(plain.contains("anthropic/claude-3-7-sonnet"));
        assert!(plain.contains("⎇ main"));
        assert!(plain.contains("14.2k tokens"));
        assert!(plain.contains("2m 14s"));
        assert!(plain.contains("$0.012"));
        assert!(plain.contains("Ready"));
    }

    #[test]
    fn test_status_bar_compact_mode() {
        let bar = StatusBar::new("claude-3-7-sonnet")
            .with_provider("anthropic")
            .with_git_branch("main")
            .with_tokens(1200)
            .with_duration(Duration::from_secs(45))
            .with_cost(0.005)
            .with_mode(StatusBarMode::Compact);

        let plain = bar.render_plain(60);
        assert!(plain.contains("claude-3-7-sonnet"));
        assert!(plain.contains("⎇ main"));
        assert!(plain.contains("1.2k tok"));
        assert!(plain.contains("45s"));
        assert!(plain.contains("$0.0050"));
    }

    #[test]
    fn test_status_bar_minimal_mode() {
        let bar = StatusBar::new("claude-3-7-sonnet")
            .with_git_branch("feat/ui-v2")
            .with_tokens(950)
            .with_duration(Duration::from_secs(30))
            .with_cost(0.001)
            .with_mode(StatusBarMode::Minimal);

        let plain = bar.render_plain(35);
        assert!(plain.contains("claude-3-7-son"));
        assert!(plain.contains("950"));
        assert!(plain.contains("30s"));
        assert!(plain.contains("$0.0010"));
    }

    #[test]
    fn test_status_bar_ansi_rendering() {
        let bar = StatusBar::new("deepseek-chat")
            .with_provider("deepseek")
            .with_git_branch("main")
            .with_tokens(5400)
            .with_duration(Duration::from_secs(12))
            .with_cost(0.0008);

        let ansi = bar.render_ansi(100, None);
        assert!(ansi.contains("\x1b["));
        assert!(ansi.contains("deepseek"));
        assert!(ansi.contains("deepseek-chat"));
        assert!(ansi.contains("5.4k"));
    }

    #[test]
    fn test_status_bar_ratatui_widget_render() {
        let backend = TestBackend::new(100, 1);
        let mut terminal = Terminal::new(backend).unwrap();

        let bar = StatusBar::new("gpt-4o")
            .with_provider("openai")
            .with_git_branch("main")
            .with_tokens(2500)
            .with_duration(Duration::from_secs(65))
            .with_cost(0.02);

        terminal
            .draw(|f| {
                let area = f.area();
                bar.render_to_frame(f, area);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let content: String = (0..100)
            .map(|x| buffer[(x, 0)].symbol().to_string())
            .collect();

        assert!(content.contains("gpt-4o"));
        assert!(content.contains("main"));
        assert!(content.contains("2.5k"));
        assert!(content.contains("1m 05s"));
    }

    #[test]
    fn test_status_bar_ratatui_widget_trait() {
        let backend = TestBackend::new(80, 2);
        let mut terminal = Terminal::new(backend).unwrap();

        let bar = StatusBar::new("claude-3-7-sonnet")
            .with_git_branch("develop")
            .with_tokens(12000)
            .with_border(true);

        terminal
            .draw(|f| {
                let area = f.area();
                f.render_widget(&bar, area);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let row1: String = (0..80)
            .map(|x| buffer[(x, 1)].symbol().to_string())
            .collect();

        assert!(row1.contains("claude-3-7-sonnet"));
        assert!(row1.contains("develop"));
    }

    #[test]
    fn test_status_bar_interoperability_with_status_info() {
        let info = StatusInfo::new("anthropic", "claude-3-5-sonnet")
            .with_agent("Coder")
            .with_advisor("SecAdvisor")
            .with_status("Indexing");

        let bar = StatusBar::from(&info).with_tokens(8400).with_cost(0.015);

        assert_eq!(bar.provider.as_deref(), Some("anthropic"));
        assert_eq!(bar.model, "claude-3-5-sonnet");
        assert_eq!(bar.active_agent.as_deref(), Some("Coder"));
        assert_eq!(bar.active_advisor.as_deref(), Some("SecAdvisor"));
        assert_eq!(bar.status_message.as_deref(), Some("Indexing"));

        let converted_back = StatusInfo::from(&bar);
        assert_eq!(converted_back.provider, "anthropic");
        assert_eq!(converted_back.model, "claude-3-5-sonnet");
        assert_eq!(converted_back.active_agent.as_deref(), Some("Coder"));
        assert_eq!(converted_back.active_advisor.as_deref(), Some("SecAdvisor"));
        assert!(converted_back.token_usage.unwrap().contains("8.4k tokens"));
    }
}
