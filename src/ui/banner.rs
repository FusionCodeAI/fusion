//! Sleek Startup ASCII Art Banner and Metadata Renderer for Fusion Code AI.
//!
//! Provides a highly polished, aesthetic terminal startup banner for Fusion:
//! - **Multiple ASCII Art Fonts & Styles**:
//!   - `Cyber`: Bold, illuminated unicode block art (`███`) with visual depth.
//!   - `Sleek`: Modern, geometric outline font with smooth aesthetic balance.
//!   - `Slant`: Flowing diagonal futuristic typography.
//!   - `Standard`: Clean, universal ASCII monospace rendering.
//!   - `Compact`: Space-efficient 3-line unicode box art for compact viewports.
//!   - `Minimal`: Ultra-clean 2-line header for tight terminals or script logs.
//!   - `OneLine`: Single-line status pill for narrow viewports or CI/CD runs.
//!   - `Auto`: Automatically selects the optimal style based on detected terminal width.
//! - **Rich Metadata & Status Badges**:
//!   - Version number (`v0.3.0`)
//!   - Active LLM Provider (DeepSeek, Anthropic, OpenAI, xAI, Ollama, OpenRouter) with branded colors
//!   - Active Model name (`deepseek-chat`, `claude-3-7-sonnet`, `gpt-4o`, etc.)
//!   - Advisor Status (Multi-Agent peer critique on/off and advisor model)
//!   - Git Branch detection & current workspace indicator
//!   - Pure-Rust engine badge and Multi-Agent Star-Mesh indicators
//! - **TrueColor RGB Multi-Stop Gradients**:
//!   - Tokyo Night (Cyan → Blue → Purple)
//!   - Monokai (Yellow → Orange → Magenta)
//!   - Dracula (Cyan → Pink → Purple)
//!   - Cyberpunk Neon (Emerald → Cyan → Neon Purple)
//!   - Sunset Glow (Coral → Rose → Gold)
//!   - High Contrast (Bright Cyan → Bright Yellow → White)
//!   - Graceful degradation to ANSI 256, ANSI 16, or Monochrome (`NO_COLOR`).
//! - **Framing & Borders**:
//!   - Rounded (`╭─╮╰─╯`), Single (`┌─┐└─┘`), Double (`╔═╗╚═╝`), Heavy (`┏━┓┗━┛`),
//!     ASCII (`+--+`), Minimal divider rules, or Borderless.
//! - **Interactive & TUI Integration**:
//!   - Standalone ANSI string generators (`render_banner_ansi`, `print_banner`)
//!   - Ratatui [`Widget`] implementation (`BannerWidget`) for embedded dashboard views.
//!   - Quick tips and keyboard shortcut hints.

use std::fmt::Write as FmtWrite;
use std::io::{stdout, Write};
use std::path::PathBuf;

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    widgets::Widget,
};
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::ui::colors::ColorCapability;
use crate::ui::table::{get_terminal_width, strip_ansi, visible_width};
use crate::ui::theme::{Theme, ThemeKind};

// ============================================================================
// 1. ASCII Art Font Definitions
// ============================================================================

/// Clean monospace wordmark (non-default; kept for parity with Sleek).
pub const BANNER_ART_CYBER: &[&str] = &[
    " ____  _   _  ____  ___  ____  _   _ ",
    "|  _ \\| | | |/ ___||_ _|/ ___|| \\ | |",
    "| |_) | | | |\\___ \\ | || |    |  \\| |",
    "|  __/| |_| | ___) || || |___ | |\\  |",
    "|_|    \\___/ |____/ |___|\\____||_| \\_|",
];

/// Clean monospace wordmark for wide terminals (new default).
pub const BANNER_ART_SLEEK: &[&str] = &[
    " ____  _   _  ____  ___  ____  _   _ ",
    "|  _ \\| | | |/ ___||_ _|/ ___|| \\ | |",
    "| |_) | | | |\\___ \\ | || |    |  \\| |",
    "|  __/| |_| | ___) || || |___ | |\\  |",
    "|_|    \\___/ |____/ |___|\\____||_| \\_|",
];

/// Slanted futuristic diagonal art.
pub const BANNER_ART_SLANT: &[&str] = &[
    "   ______           _             ",
    "  / ____/_  _______(_)___  ____   ",
    " / /_  / / / / ___/ / __ \\/ __ \\  ",
    "/ __/ / /_/ (__  ) / /_/ / / / /  ",
    "/_/    \\__,_/____/_/\\____/_/ /_/   ",
];

/// Standard clean ASCII monospace art.
pub const BANNER_ART_STANDARD: &[&str] = &[
    " ____ _  _ ____ _ ____ _  _ ",
    " |___ |  | [__  | |  | |\\ | ",
    " |    |__| ___] | |__| | \\| ",
];

/// Compact 3-line unicode box art for mobile / Termux / narrow terminals.
pub const BANNER_ART_COMPACT: &[&str] =
    &["┌─┐┬ ┬┌─┐┬┌─┐┌┐┌", "├┤ │ │└─┐││ ││││", "└  └─┘└─┘┴└─┘┘└┘"];

/// Subtitle text for Fusion Code AI.
pub const DEFAULT_SUBTITLE: &str = "PURE-RUST AI CODING ASSISTANT";

/// Quick startup tips rotated or displayed in banner.
pub const QUICK_TIPS: &[&str] = &[
    "Type your prompt or /help for command palette",
    "Use /model to switch active LLM provider or model",
    "Use /subagents to inspect the multi-agent execution mesh",
    "Use /compact to prune and compress conversation context",
    "Use /cost to view session token usage and USD expenditure",
    "Use /doctor to diagnose API keys and network connectivity",
    "Press Ctrl+C to cancel turn or Ctrl+D / /exit to quit",
];

// ============================================================================
// 2. Banner Enums & Configurations
// ============================================================================

/// ASCII Art visual style for the banner logo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum BannerStyle {
    /// Bold, illuminated unicode block art (`███`). Width ~47 cols.
    Cyber,
    /// Modern geometric outline font. Width ~34 cols.
    Sleek,
    /// Flowing diagonal futuristic font. Width ~34 cols.
    Slant,
    /// Clean universal ASCII art. Width ~28 cols.
    Standard,
    /// Space-efficient 3-line unicode box art. Width ~16 cols.
    Compact,
    /// Ultra-clean 2-line textual header without large art.
    Minimal,
    /// Single-line status badge.
    OneLine,
    /// Automatically choose best style based on terminal width.
    #[default]
    Auto,
}

impl BannerStyle {
    /// Return the raw ASCII art lines for this style.
    pub fn lines(&self) -> &'static [&'static str] {
        match self {
            Self::Cyber => BANNER_ART_CYBER,
            Self::Sleek => BANNER_ART_SLEEK,
            Self::Slant => BANNER_ART_SLANT,
            Self::Standard => BANNER_ART_STANDARD,
            Self::Compact => BANNER_ART_COMPACT,
            Self::Minimal | Self::OneLine => &[],
            Self::Auto => BANNER_ART_CYBER,
        }
    }

    /// Return the maximum character width of the ASCII art lines.
    pub fn art_width(&self) -> usize {
        self.lines()
            .iter()
            .map(|l| visible_width(l))
            .max()
            .unwrap_or(0)
    }

    /// Resolve `Auto` to a concrete `BannerStyle` given the available width.
    pub fn resolve_for_width(&self, width: usize) -> Self {
        if !matches!(self, Self::Auto) {
            return *self;
        }

        if width >= 78 {
            Self::Sleek
        } else if width >= 58 {
            Self::Standard
        } else if width >= 44 {
            Self::Compact
        } else if width >= 32 {
            Self::Minimal
        } else {
            Self::OneLine
        }
    }
}

/// Border framing style around the startup banner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum BannerBoxBorder {
    /// No outer border box.
    #[default]
    None,
    /// Rounded corners (`╭─╮╰─╯`).
    Rounded,
    /// Single line border (`┌─┐└─┘`).
    Single,
    /// Double line border (`╔═╗╚═╝`).
    Double,
    /// Heavy/thick line border (`┏━┓┗━┛`).
    Heavy,
    /// Plain ASCII box (`+--+`).
    Ascii,
    /// Horizontal top and bottom divider lines only.
    HorizontalRules,
    /// Left-side vertical accent bar only (`▌` or `│`).
    LeftAccentBar,
}

impl BannerBoxBorder {
    /// Returns box border characters: `(top_left, top_right, bot_left, bot_right, horiz, vert)`.
    pub fn chars(&self) -> (char, char, char, char, char, char) {
        match self {
            Self::None => (' ', ' ', ' ', ' ', ' ', ' '),
            Self::Rounded => ('╭', '╮', '╰', '╯', '─', '│'),
            Self::Single => ('┌', '┐', '└', '┘', '─', '│'),
            Self::Double => ('╔', '╗', '╚', '╝', '═', '║'),
            Self::Heavy => ('┏', '┓', '┗', '┛', '━', '┃'),
            Self::Ascii => ('+', '+', '+', '+', '-', '|'),
            Self::HorizontalRules => ('─', '─', '─', '─', '─', ' '),
            Self::LeftAccentBar => ('▌', ' ', '▌', ' ', ' ', '▌'),
        }
    }
}

/// Color rendering mode for the banner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum BannerColorMode {
    /// Auto-detect color capability from environment.
    #[default]
    Auto,
    /// TrueColor RGB continuous gradient.
    TrueColorGradient,
    /// Theme-based discrete ANSI colors.
    Themed,
    /// 256-color palette.
    Ansi256,
    /// Standard 16-color ANSI palette.
    Ansi16,
    /// Monochrome plaintext (no ANSI color codes).
    Monochrome,
}

/// Gradient color theme preset for the banner ASCII art.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum GradientPreset {
    /// Tokyo Night (Cyan `#7dcfff` -> Blue `#7aa2f7` -> Purple `#bb9af7`).
    #[default]
    TokyoNight,
    /// Monokai (Yellow `#e6db74` -> Orange `#fd971f` -> Magenta `#f92672`).
    Monokai,
    /// Dracula (Cyan `#8be9fd` -> Pink `#ff79c6` -> Purple `#bd93f9`).
    Dracula,
    /// Cyberpunk Neon (Emerald `#00ff9f` -> Electric Cyan `#00b8ff` -> Purple `#d600ff`).
    CyberNeon,
    /// Sunset Glow (Coral `#ff7a59` -> Rose `#ff4e50` -> Amber `#f9d423`).
    Sunset,
    /// Emerald Forest (Teal `#1abc9c` -> Emerald `#2ecc71` -> Mint `#a8e6cf`).
    Emerald,
    /// High Contrast (Cyan `#00ffff` -> Yellow `#ffff00` -> White `#ffffff`).
    HighContrast,
}

impl GradientPreset {
    /// Get RGB color stops for this gradient preset: `(position_factor, (r, g, b))`.
    pub fn stops(&self) -> &'static [(f32, (u8, u8, u8))] {
        match self {
            Self::TokyoNight => &[
                (0.0, (125, 207, 255)), // Cyan #7dcfff
                (0.5, (122, 162, 247)), // Blue #7aa2f7
                (1.0, (187, 154, 247)), // Purple #bb9af7
            ],
            Self::Monokai => &[
                (0.0, (230, 219, 116)), // Yellow #e6db74
                (0.5, (253, 151, 31)),  // Orange #fd971f
                (1.0, (249, 38, 114)),  // Magenta #f92672
            ],
            Self::Dracula => &[
                (0.0, (139, 233, 253)), // Cyan #8be9fd
                (0.5, (255, 121, 198)), // Pink #ff79c6
                (1.0, (189, 147, 249)), // Purple #bd93f9
            ],
            Self::CyberNeon => &[
                (0.0, (0, 255, 159)), // Emerald #00ff9f
                (0.5, (0, 184, 255)), // Cyan #00b8ff
                (1.0, (214, 0, 255)), // Purple #d600ff
            ],
            Self::Sunset => &[
                (0.0, (255, 122, 89)), // Coral #ff7a59
                (0.5, (255, 78, 80)),  // Rose #ff4e50
                (1.0, (249, 212, 35)), // Amber #f9d423
            ],
            Self::Emerald => &[
                (0.0, (26, 188, 156)),  // Teal #1abc9c
                (0.5, (46, 204, 113)),  // Emerald #2ecc71
                (1.0, (168, 230, 207)), // Mint #a8e6cf
            ],
            Self::HighContrast => &[
                (0.0, (0, 255, 255)),   // Cyan #00ffff
                (0.5, (255, 255, 0)),   // Yellow #ffff00
                (1.0, (255, 255, 255)), // White #ffffff
            ],
        }
    }

    /// Select gradient matching a `ThemeKind`.
    pub fn from_theme_kind(kind: ThemeKind) -> Self {
        match kind {
            ThemeKind::TokyoNight => Self::TokyoNight,
            ThemeKind::Monokai => Self::Monokai,
            ThemeKind::Dracula => Self::Dracula,
            ThemeKind::HighContrast => Self::HighContrast,
            ThemeKind::Adaptive => Self::TokyoNight,
        }
    }
}

// ============================================================================
// 3. Banner Info & Configuration Models
// ============================================================================

/// Metadata details displayed within the startup banner.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BannerInfo {
    /// Application version string (e.g. `0.3.0` or `v0.3.0`).
    pub version: String,
    /// Active LLM provider name (e.g. `deepseek`, `anthropic`, `openai`, `ollama`).
    pub provider: String,
    /// Active LLM model name (e.g. `deepseek-chat`, `claude-3-7-sonnet`).
    pub model: String,
    /// Whether peer advisor critiques are enabled.
    pub advisors_enabled: bool,
    /// Specific advisor model override if configured.
    pub advisor_model: Option<String>,
    /// Current workspace directory path.
    pub workspace_dir: Option<PathBuf>,
    /// Active Git branch name if inside a git repository.
    pub git_branch: Option<String>,
    /// Context window limit in tokens.
    pub context_limit: Option<usize>,
    /// Configured sampling temperature.
    pub temperature: Option<f32>,
    /// Max tokens per generation.
    pub max_tokens: Option<u32>,
    /// Whether audio sound cues are enabled.
    pub sound_enabled: bool,
    /// Whether desktop notifications are enabled.
    pub notify_enabled: bool,
    /// Whether the engine is running in pure-Rust zero-dependency mode.
    pub pure_rust: bool,
    /// Custom subtitle / tagline override.
    pub custom_tagline: Option<String>,
    /// Custom arbitrary key-value badges `(label, value)`.
    pub custom_badges: Vec<(String, String)>,
}

impl Default for BannerInfo {
    fn default() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_string(),
            provider: "deepseek".to_string(),
            model: "deepseek-chat".to_string(),
            advisors_enabled: true,
            advisor_model: None,
            workspace_dir: std::env::current_dir().ok(),
            git_branch: detect_git_branch(),
            context_limit: Some(128_000),
            temperature: None,
            max_tokens: None,
            sound_enabled: false,
            notify_enabled: true,
            pure_rust: true,
            custom_tagline: None,
            custom_badges: Vec::new(),
        }
    }
}

impl BannerInfo {
    /// Create a new `BannerInfo` with minimal required fields.
    pub fn new(
        version: impl Into<String>,
        provider: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            version: version.into(),
            provider: provider.into(),
            model: model.into(),
            ..Default::default()
        }
    }

    /// Construct `BannerInfo` populated from a Fusion `Config`.
    pub fn from_config(config: &Config) -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_string(),
            provider: config.default_provider.clone(),
            model: config.default_model.clone(),
            advisors_enabled: config.advisors_enabled,
            advisor_model: config.advisor_model.clone(),
            workspace_dir: std::env::current_dir().ok(),
            git_branch: detect_git_branch(),
            context_limit: None,
            temperature: config.default_temperature,
            max_tokens: config.max_tokens,
            sound_enabled: config.sound_enabled,
            notify_enabled: config.notify_enabled,
            pure_rust: true,
            custom_tagline: None,
            custom_badges: Vec::new(),
        }
    }

    /// Create a fluent builder for `BannerInfo`.
    pub fn builder() -> BannerInfoBuilder {
        BannerInfoBuilder::default()
    }

    /// Formatted version string with `v` prefix.
    pub fn version_display(&self) -> String {
        if self.version.starts_with('v') || self.version.starts_with('V') {
            self.version.clone()
        } else {
            format!("v{}", self.version)
        }
    }

    /// Canonicalized, capitalized provider display name.
    pub fn provider_display(&self) -> &str {
        match self.provider.to_ascii_lowercase().as_str() {
            "deepseek" => "DeepSeek",
            "anthropic" => "Anthropic",
            "openai" => "OpenAI",
            "xai" => "xAI / Grok",
            "openrouter" => "OpenRouter",
            "ollama" => "Ollama (Local)",
            _ => &self.provider,
        }
    }

    /// Advisor status summary text.
    pub fn advisors_display(&self) -> String {
        if self.advisors_enabled {
            if let Some(model) = &self.advisor_model {
                format!("on ({})", model)
            } else {
                "on".to_string()
            }
        } else {
            "off".to_string()
        }
    }
}

/// Fluent builder for [`BannerInfo`].
#[derive(Debug, Clone, Default)]
pub struct BannerInfoBuilder {
    info: BannerInfo,
}

impl BannerInfoBuilder {
    pub fn version(mut self, version: impl Into<String>) -> Self {
        self.info.version = version.into();
        self
    }

    pub fn provider(mut self, provider: impl Into<String>) -> Self {
        self.info.provider = provider.into();
        self
    }

    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.info.model = model.into();
        self
    }

    pub fn advisors_enabled(mut self, enabled: bool) -> Self {
        self.info.advisors_enabled = enabled;
        self
    }

    pub fn advisor_model(mut self, model: Option<String>) -> Self {
        self.info.advisor_model = model;
        self
    }

    pub fn workspace_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.info.workspace_dir = Some(path.into());
        self
    }

    pub fn git_branch(mut self, branch: Option<String>) -> Self {
        self.info.git_branch = branch;
        self
    }

    pub fn context_limit(mut self, limit: Option<usize>) -> Self {
        self.info.context_limit = limit;
        self
    }

    pub fn temperature(mut self, temp: Option<f32>) -> Self {
        self.info.temperature = temp;
        self
    }

    pub fn max_tokens(mut self, tokens: Option<u32>) -> Self {
        self.info.max_tokens = tokens;
        self
    }

    pub fn sound_enabled(mut self, enabled: bool) -> Self {
        self.info.sound_enabled = enabled;
        self
    }

    pub fn notify_enabled(mut self, enabled: bool) -> Self {
        self.info.notify_enabled = enabled;
        self
    }

    pub fn pure_rust(mut self, pure: bool) -> Self {
        self.info.pure_rust = pure;
        self
    }

    pub fn custom_tagline(mut self, tagline: impl Into<String>) -> Self {
        self.info.custom_tagline = Some(tagline.into());
        self
    }

    pub fn add_badge(mut self, label: impl Into<String>, value: impl Into<String>) -> Self {
        self.info.custom_badges.push((label.into(), value.into()));
        self
    }

    pub fn build(self) -> BannerInfo {
        self.info
    }
}

/// Rendering options and styling flags for banner output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BannerConfig {
    /// Visual style for ASCII logo.
    pub style: BannerStyle,
    /// Box framing style.
    pub box_border: BannerBoxBorder,
    /// Color mode.
    pub color_mode: BannerColorMode,
    /// Gradient preset for TrueColor rendering.
    pub gradient: GradientPreset,
    /// Whether to display metadata rows (provider, model, advisors).
    pub show_metadata: bool,
    /// Whether to display the tagline/subtitle.
    pub show_tagline: bool,
    /// Whether to display quick tips or shortcuts.
    pub show_tips: bool,
    /// Whether to display system / workspace badges (git branch, cwd).
    pub show_system_info: bool,
    /// Specific quick tip to show (if None, shows default).
    pub tip_override: Option<String>,
    /// Explicit width constraint (overrides terminal auto-detection).
    pub custom_width: Option<usize>,
    /// Center align the ASCII art logo.
    pub center_logo: bool,
    /// Left padding margin in spaces.
    pub left_margin: usize,
}

impl Default for BannerConfig {
    fn default() -> Self {
        Self {
            style: BannerStyle::Auto,
            box_border: BannerBoxBorder::None,
            color_mode: BannerColorMode::Auto,
            gradient: GradientPreset::TokyoNight,
            show_metadata: true,
            show_tagline: true,
            show_tips: true,
            show_system_info: true,
            tip_override: None,
            custom_width: None,
            center_logo: false,
            left_margin: 2,
        }
    }
}

impl BannerConfig {
    /// Create a compact banner configuration suitable for narrow viewports or CI/CD.
    pub fn compact() -> Self {
        Self {
            style: BannerStyle::Compact,
            box_border: BannerBoxBorder::None,
            show_tips: false,
            show_system_info: false,
            left_margin: 1,
            ..Default::default()
        }
    }

    /// Create a minimal 2-line header configuration.
    pub fn minimal() -> Self {
        Self {
            style: BannerStyle::Minimal,
            box_border: BannerBoxBorder::None,
            show_tips: false,
            show_system_info: false,
            left_margin: 1,
            ..Default::default()
        }
    }

    /// Create a framed aesthetic box banner configuration.
    pub fn framed(border: BannerBoxBorder) -> Self {
        Self {
            box_border: border,
            left_margin: 0,
            ..Default::default()
        }
    }

    /// Fluent builder for [`BannerConfig`].
    pub fn builder() -> BannerConfigBuilder {
        BannerConfigBuilder::default()
    }
}

/// Fluent builder for [`BannerConfig`].
#[derive(Debug, Clone, Default)]
pub struct BannerConfigBuilder {
    config: BannerConfig,
}

impl BannerConfigBuilder {
    pub fn style(mut self, style: BannerStyle) -> Self {
        self.config.style = style;
        self
    }

    pub fn box_border(mut self, border: BannerBoxBorder) -> Self {
        self.config.box_border = border;
        self
    }

    pub fn color_mode(mut self, mode: BannerColorMode) -> Self {
        self.config.color_mode = mode;
        self
    }

    pub fn gradient(mut self, gradient: GradientPreset) -> Self {
        self.config.gradient = gradient;
        self
    }

    pub fn show_metadata(mut self, show: bool) -> Self {
        self.config.show_metadata = show;
        self
    }

    pub fn show_tagline(mut self, show: bool) -> Self {
        self.config.show_tagline = show;
        self
    }

    pub fn show_tips(mut self, show: bool) -> Self {
        self.config.show_tips = show;
        self
    }

    pub fn show_system_info(mut self, show: bool) -> Self {
        self.config.show_system_info = show;
        self
    }

    pub fn tip(mut self, tip: impl Into<String>) -> Self {
        self.config.tip_override = Some(tip.into());
        self
    }

    pub fn width(mut self, width: usize) -> Self {
        self.config.custom_width = Some(width);
        self
    }

    pub fn center_logo(mut self, center: bool) -> Self {
        self.config.center_logo = center;
        self
    }

    pub fn left_margin(mut self, margin: usize) -> Self {
        self.config.left_margin = margin;
        self
    }

    pub fn build(self) -> BannerConfig {
        self.config
    }
}

// ============================================================================
// 4. Color & Gradient Math Helpers
// ============================================================================

/// Linearly interpolate between two RGB colors by factor `t` in `[0.0, 1.0]`.
pub fn interpolate_rgb(c1: (u8, u8, u8), c2: (u8, u8, u8), t: f32) -> (u8, u8, u8) {
    let t = t.clamp(0.0, 1.0);
    let r = (c1.0 as f32 + (c2.0 as f32 - c1.0 as f32) * t).round() as u8;
    let g = (c1.1 as f32 + (c2.1 as f32 - c1.1 as f32) * t).round() as u8;
    let b = (c1.2 as f32 + (c2.2 as f32 - c1.2 as f32) * t).round() as u8;
    (r, g, b)
}

/// Compute RGB color along a multi-stop color gradient ramp.
pub fn multi_stop_gradient(stops: &[(f32, (u8, u8, u8))], factor: f32) -> (u8, u8, u8) {
    if stops.is_empty() {
        return (255, 255, 255);
    }
    if stops.len() == 1 {
        return stops[0].1;
    }

    let factor = factor.clamp(0.0, 1.0);

    // Find bounding stop segment
    for i in 0..stops.len() - 1 {
        let (p1, c1) = stops[i];
        let (p2, c2) = stops[i + 1];
        if factor >= p1 && factor <= p2 {
            let seg_len = (p2 - p1).max(0.0001);
            let seg_t = (factor - p1) / seg_len;
            return interpolate_rgb(c1, c2, seg_t);
        }
    }

    stops.last().unwrap().1
}

/// Convert 24-bit direct RGB color to an ANSI TrueColor escape sequence.
#[inline]
pub fn ansi_rgb_fg(r: u8, g: u8, b: u8) -> String {
    format!("\x1b[38;2;{};{};{}m", r, g, b)
}

/// Format a single character with TrueColor RGB.
#[inline]
pub fn format_char_rgb(c: char, r: u8, g: u8, b: u8) -> String {
    format!("\x1b[38;2;{};{};{}m{}\x1b[0m", r, g, b, c)
}

/// Formats a line of text applying a continuous horizontal gradient across its characters.
pub fn apply_horizontal_gradient(text: &str, stops: &[(f32, (u8, u8, u8))]) -> String {
    let char_count = text.chars().count();
    if char_count == 0 {
        return String::new();
    }

    let mut out = String::with_capacity(text.len() * 20);
    let mut current_idx = 0;

    for c in text.chars() {
        if c == ' ' {
            out.push(' ');
        } else {
            let factor = if char_count > 1 {
                current_idx as f32 / (char_count - 1) as f32
            } else {
                0.0
            };
            let (r, g, b) = multi_stop_gradient(stops, factor);
            let _ = write!(out, "\x1b[38;2;{};{};{}m{}\x1b[0m", r, g, b, c);
        }
        current_idx += 1;
    }

    out
}

/// Formats a block of ASCII art applying a unified 2D diagonal gradient across all rows.
pub fn apply_diagonal_gradient(lines: &[&str], stops: &[(f32, (u8, u8, u8))]) -> Vec<String> {
    if lines.is_empty() {
        return Vec::new();
    }

    let num_rows = lines.len();
    let max_cols = lines
        .iter()
        .map(|l| l.chars().count())
        .max()
        .unwrap_or(1)
        .max(1);

    lines
        .iter()
        .enumerate()
        .map(|(row_idx, line)| {
            let mut out = String::with_capacity(line.len() * 20);
            for (col_idx, c) in line.chars().enumerate() {
                if c == ' ' {
                    out.push(' ');
                } else {
                    // Normalize (x, y) coordinates diagonally (top-left to bottom-right)
                    let norm_x = col_idx as f32 / max_cols as f32;
                    let norm_y = row_idx as f32 / num_rows.max(1) as f32;
                    let factor = (norm_x * 0.75 + norm_y * 0.25).clamp(0.0, 1.0);

                    let (r, g, b) = multi_stop_gradient(stops, factor);
                    let _ = write!(out, "\x1b[38;2;{};{};{}m{}\x1b[0m", r, g, b, c);
                }
            }
            out
        })
        .collect()
}

// ============================================================================
// 5. Standalone ANSI Banner Rendering Engine
// ============================================================================

/// Render a complete startup banner to a string using ANSI escape sequences.
pub fn render_banner(info: &BannerInfo, config: &BannerConfig) -> String {
    let term_width = config.custom_width.unwrap_or_else(get_terminal_width);
    let resolved_style = config.style.resolve_for_width(term_width);
    let color_cap = resolve_effective_color_mode(config.color_mode);

    let mut out = String::new();

    // Box top border if framed
    if config.box_border != BannerBoxBorder::None {
        render_box_top(&mut out, config.box_border, term_width, color_cap);
    }

    // Left margin indentation
    let pad_str = " ".repeat(config.left_margin);

    // 1. Render ASCII Art Logo
    match resolved_style {
        BannerStyle::Minimal => {
            render_minimal_header(&mut out, info, config, &pad_str, color_cap);
        }
        BannerStyle::OneLine => {
            render_oneline_header(&mut out, info, config, &pad_str, color_cap);
        }
        _ => {
            render_art_logo(
                &mut out,
                info,
                config,
                resolved_style,
                &pad_str,
                color_cap,
                term_width,
            );
        }
    }

    // 2. Render Subtitle & Tagline
    if config.show_tagline
        && resolved_style != BannerStyle::OneLine
        && resolved_style != BannerStyle::Minimal
    {
        render_tagline(&mut out, info, config, &pad_str, color_cap);
    }

    // 3. Render Metadata Pills / Badges (Provider, Model, Advisors, Version)
    if config.show_metadata && resolved_style != BannerStyle::OneLine {
        render_metadata_block(&mut out, info, config, &pad_str, color_cap, term_width);
    }

    // 4. Render System & Workspace Info (Git Branch, Dir)
    if config.show_system_info
        && resolved_style != BannerStyle::OneLine
        && resolved_style != BannerStyle::Minimal
    {
        render_system_info_block(&mut out, info, config, &pad_str, color_cap, term_width);
    }

    // 5. Render Quick Tip / Command Hints
    if config.show_tips && resolved_style != BannerStyle::OneLine {
        render_tips_block(&mut out, info, config, &pad_str, color_cap);
    }

    // Box bottom border if framed
    if config.box_border != BannerBoxBorder::None {
        render_box_bottom(&mut out, config.box_border, term_width, color_cap);
    }

    out
}

/// Render the ASCII art logo lines with color gradient.
fn render_art_logo(
    out: &mut String,
    info: &BannerInfo,
    config: &BannerConfig,
    style: BannerStyle,
    pad: &str,
    color_cap: ColorCapability,
    term_width: usize,
) {
    let art_lines = style.lines();
    if art_lines.is_empty() {
        return;
    }

    out.push('\n');

    let art_width = style.art_width();
    let center_pad = if config.center_logo && term_width > art_width + config.left_margin {
        let available = term_width.saturating_sub(art_width);
        " ".repeat(available / 2)
    } else {
        pad.to_string()
    };

    if color_cap >= ColorCapability::TrueColor {
        let gradient_lines = apply_diagonal_gradient(art_lines, config.gradient.stops());
        for line in gradient_lines {
            let _ = writeln!(out, "{}{}", center_pad, line);
        }
    } else if color_cap.has_color() {
        // ANSI 16 or 256 colors: vibrant cyan/magenta
        for (i, line) in art_lines.iter().enumerate() {
            let color_code = if i % 2 == 0 {
                "\x1b[1;36m"
            } else {
                "\x1b[1;35m"
            };
            let _ = writeln!(out, "{}{}{}\x1b[0m", center_pad, color_code, line);
        }
    } else {
        // Monochrome
        for line in art_lines {
            let _ = writeln!(out, "{}{}", center_pad, line);
        }
    }
}

/// Render the subtitle and tagline under the logo.
fn render_tagline(
    out: &mut String,
    info: &BannerInfo,
    _config: &BannerConfig,
    pad: &str,
    color_cap: ColorCapability,
) {
    let tagline = info.custom_tagline.as_deref().unwrap_or(DEFAULT_SUBTITLE);

    if color_cap.has_color() {
        let _ = writeln!(
            out,
            "{}\x1b[2;37m✦\x1b[0m \x1b[1;37m{}\x1b[0m \x1b[1;36m{}\x1b[0m \x1b[2;37m✦\x1b[0m",
            pad,
            tagline,
            info.version_display()
        );
    } else {
        let _ = writeln!(out, "{}✦ {} {} ✦", pad, tagline, info.version_display());
    }
}

/// Render the structured metadata block (Provider, Model, Advisors, etc.).
fn render_metadata_block(
    out: &mut String,
    info: &BannerInfo,
    _config: &BannerConfig,
    pad: &str,
    color_cap: ColorCapability,
    term_width: usize,
) {
    let is_narrow = term_width < 65;

    let provider_color = match info.provider.to_ascii_lowercase().as_str() {
        "deepseek" => "\x1b[1;34m",   // Blue
        "anthropic" => "\x1b[1;33m",  // Warm Amber
        "openai" => "\x1b[1;32m",     // Green
        "xai" => "\x1b[1;37m",        // White
        "ollama" => "\x1b[1;36m",     // Cyan
        "openrouter" => "\x1b[1;35m", // Purple
        _ => "\x1b[1;33m",
    };

    let advisor_color = if info.advisors_enabled {
        "\x1b[1;32m" // Green
    } else {
        "\x1b[1;31m" // Red
    };

    if color_cap.has_color() {
        if is_narrow {
            // Multi-line compact metadata
            let _ = writeln!(
                out,
                "{}\x1b[2;37mProvider:\x1b[0m {}{}\x1b[0m  \x1b[2;37mModel:\x1b[0m \x1b[1;37m{}\x1b[0m",
                pad,
                provider_color,
                info.provider_display(),
                info.model
            );
            let _ = writeln!(
                out,
                "{}\x1b[2;37mAdvisors:\x1b[0m {}{}\x1b[0m  \x1b[2;37mEngine:\x1b[0m \x1b[1;36mPure-Rust ⚡\x1b[0m",
                pad,
                advisor_color,
                info.advisors_display()
            );
        } else {
            // Single sleek horizontal status bar
            let _ = writeln!(
                out,
                "{}\x1b[2;37mProvider:\x1b[0m {}{}\x1b[0m  \x1b[2;37mModel:\x1b[0m \x1b[1;37m{}\x1b[0m  \x1b[2;37mAdvisors:\x1b[0m {}{}\x1b[0m  \x1b[2;37mEngine:\x1b[0m \x1b[1;36mPure-Rust ⚡\x1b[0m",
                pad,
                provider_color,
                info.provider_display(),
                info.model,
                advisor_color,
                info.advisors_display()
            );
        }
    } else {
        // Monochrome
        if is_narrow {
            let _ = writeln!(
                out,
                "{}Provider: {}  Model: {}",
                pad,
                info.provider_display(),
                info.model
            );
            let _ = writeln!(
                out,
                "{}Advisors: {}  Engine: Pure-Rust",
                pad,
                info.advisors_display()
            );
        } else {
            let _ = writeln!(
                out,
                "{}Provider: {}  Model: {}  Advisors: {}  Engine: Pure-Rust",
                pad,
                info.provider_display(),
                info.model,
                info.advisors_display()
            );
        }
    }
}

/// Render workspace and git repository info.
fn render_system_info_block(
    out: &mut String,
    info: &BannerInfo,
    _config: &BannerConfig,
    pad: &str,
    color_cap: ColorCapability,
    _term_width: usize,
) {
    let mut parts = Vec::new();
    if let Some(branch) = &info.git_branch {
        if color_cap.has_color() {
            parts.push(format!(
                "\x1b[2;37mBranch:\x1b[0m \x1b[1;35m⎇ {}\x1b[0m",
                branch
            ));
        } else {
            parts.push(format!("Branch: {}", branch));
        }
    }

    if let Some(dir) = &info.workspace_dir {
        let dir_str = dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_else(|| dir.to_str().unwrap_or("."));

        if color_cap.has_color() {
            parts.push(format!(
                "\x1b[2;37mWorkspace:\x1b[0m \x1b[1;34m📁 {}\x1b[0m",
                dir_str
            ));
        } else {
            parts.push(format!("Workspace: {}", dir_str));
        }
    }

    for (label, value) in &info.custom_badges {
        if color_cap.has_color() {
            parts.push(format!(
                "\x1b[2;37m{}:\x1b[0m \x1b[1;36m{}\x1b[0m",
                label, value
            ));
        } else {
            parts.push(format!("{}: {}", label, value));
        }
    }

    if !parts.is_empty() {
        let line = parts.join("  ");
        let _ = writeln!(out, "{}{}", pad, line);
    }
}

/// Render interactive tips and command hints.
fn render_tips_block(
    out: &mut String,
    _info: &BannerInfo,
    config: &BannerConfig,
    pad: &str,
    color_cap: ColorCapability,
) {
    let tip = config
        .tip_override
        .as_deref()
        .unwrap_or("Type prompt, /help for commands, /model to switch, or Ctrl+D to quit.");

    if color_cap.has_color() {
        let _ = writeln!(out, "{}\x1b[2;37mTip:\x1b[0m \x1b[36m{}\x1b[0m", pad, tip);
    } else {
        let _ = writeln!(out, "{}Tip: {}", pad, tip);
    }
}

/// Render minimal 2-line header for tight terminals.
fn render_minimal_header(
    out: &mut String,
    info: &BannerInfo,
    _config: &BannerConfig,
    pad: &str,
    color_cap: ColorCapability,
) {
    if color_cap.has_color() {
        let _ = writeln!(
            out,
            "{}\x1b[1;36m✦ Fusion {}\x1b[0m \x1b[2;37m(Pure-Rust AI Coding Assistant)\x1b[0m",
            pad,
            info.version_display()
        );
    } else {
        let _ = writeln!(
            out,
            "{}✦ Fusion {} (Pure-Rust AI Coding Assistant)",
            pad,
            info.version_display()
        );
    }
}

/// Render single-line status pill for narrow viewports or CI/CD.
fn render_oneline_header(
    out: &mut String,
    info: &BannerInfo,
    _config: &BannerConfig,
    pad: &str,
    color_cap: ColorCapability,
) {
    if color_cap.has_color() {
        let _ = writeln!(
            out,
            "{}\x1b[1;36m✦ Fusion {}\x1b[0m \x1b[2;37m[\x1b[0m\x1b[1;33m{}\x1b[0m\x1b[2;37m:\x1b[0m\x1b[1;37m{}\x1b[0m\x1b[2;37m]\x1b[0m",
            pad,
            info.version_display(),
            info.provider_display(),
            info.model
        );
    } else {
        let _ = writeln!(
            out,
            "{}✦ Fusion {} [{}:{}]",
            pad,
            info.version_display(),
            info.provider_display(),
            info.model
        );
    }
}

/// Render top border line of framed box.
fn render_box_top(
    out: &mut String,
    border: BannerBoxBorder,
    width: usize,
    color_cap: ColorCapability,
) {
    let (tl, tr, _, _, h, _) = border.chars();
    let inner_width = width.saturating_sub(2).max(10);
    let bar: String = std::iter::repeat(h).take(inner_width).collect();

    if color_cap.has_color() {
        let _ = writeln!(out, "\x1b[2;36m{}{}{}\x1b[0m", tl, bar, tr);
    } else {
        let _ = writeln!(out, "{}{}{}", tl, bar, tr);
    }
}

/// Render bottom border line of framed box.
fn render_box_bottom(
    out: &mut String,
    border: BannerBoxBorder,
    width: usize,
    color_cap: ColorCapability,
) {
    let (_, _, bl, br, h, _) = border.chars();
    let inner_width = width.saturating_sub(2).max(10);
    let bar: String = std::iter::repeat(h).take(inner_width).collect();

    if color_cap.has_color() {
        let _ = writeln!(out, "\x1b[2;36m{}{}{}\x1b[0m", bl, bar, br);
    } else {
        let _ = writeln!(out, "{}{}{}", bl, bar, br);
    }
}

/// Resolve effective `ColorCapability` from user request and terminal environment.
fn resolve_effective_color_mode(mode: BannerColorMode) -> ColorCapability {
    match mode {
        BannerColorMode::Auto => ColorCapability::detect(),
        BannerColorMode::TrueColorGradient => ColorCapability::TrueColor,
        BannerColorMode::Themed => ColorCapability::Ansi256,
        BannerColorMode::Ansi256 => ColorCapability::Ansi256,
        BannerColorMode::Ansi16 => ColorCapability::Ansi16,
        BannerColorMode::Monochrome => ColorCapability::NoColor,
    }
}

// ============================================================================
// 6. Helper Functions & Git Branch Detection
// ============================================================================

/// Detect current Git branch from `.git/HEAD` or environment without external commands.
pub fn detect_git_branch() -> Option<String> {
    // 1. Check CI/CD environment variables
    for var in &[
        "GIT_BRANCH",
        "GITHUB_HEAD_REF",
        "GITHUB_REF_NAME",
        "CI_COMMIT_BRANCH",
        "GIT_LOCAL_BRANCH",
    ] {
        if let Ok(val) = std::env::var(var) {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }

    // 2. Search upwards for `.git/HEAD`
    let mut curr = std::env::current_dir().ok()?;
    loop {
        let head_path = curr.join(".git").join("HEAD");
        if head_path.is_file() {
            if let Ok(content) = std::fs::read_to_string(&head_path) {
                let trimmed = content.trim();
                if let Some(ref_path) = trimmed.strip_prefix("ref: refs/heads/") {
                    return Some(ref_path.to_string());
                } else if trimmed.len() >= 7 {
                    // Detached HEAD commit hash short representation
                    return Some(trimmed[..7].to_string());
                }
            }
            break;
        }

        if !curr.pop() {
            break;
        }
    }

    None
}

// ============================================================================
// 7. Ratatui Widget Implementation
// ============================================================================

/// Ratatui [`Widget`] rendering the Fusion startup banner in TUI applications.
#[derive(Debug, Clone)]
pub struct BannerWidget<'a> {
    info: &'a BannerInfo,
    config: &'a BannerConfig,
    theme: Option<&'a Theme>,
}

impl<'a> BannerWidget<'a> {
    /// Create a new `BannerWidget` with the given info and config.
    pub fn new(info: &'a BannerInfo, config: &'a BannerConfig) -> Self {
        Self {
            info,
            config,
            theme: None,
        }
    }

    /// Set an explicit theme for the widget.
    pub fn theme(mut self, theme: &'a Theme) -> Self {
        self.theme = Some(theme);
        self
    }
}

impl<'a> Widget for BannerWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 10 || area.height < 2 {
            return;
        }

        let resolved_style = self.config.style.resolve_for_width(area.width as usize);
        let default_theme = Theme::tokyo_night();
        let theme = self.theme.unwrap_or(&default_theme);

        let mut current_y = area.y;

        // 1. Render ASCII Art lines
        let art_lines = resolved_style.lines();
        if !art_lines.is_empty() && current_y < area.bottom() {
            let stops = self.config.gradient.stops();
            let max_cols = art_lines
                .iter()
                .map(|l| l.chars().count())
                .max()
                .unwrap_or(1);
            let num_rows = art_lines.len();

            for (row_idx, line) in art_lines.iter().enumerate() {
                if current_y >= area.bottom() {
                    break;
                }

                let x_start = if self.config.center_logo {
                    let line_w = line.chars().count() as u16;
                    area.x + area.width.saturating_sub(line_w) / 2
                } else {
                    area.x + self.config.left_margin as u16
                };

                for (col_idx, c) in line.chars().enumerate() {
                    let cell_x = x_start + col_idx as u16;
                    if cell_x < area.right() && c != ' ' {
                        let norm_x = col_idx as f32 / max_cols as f32;
                        let norm_y = row_idx as f32 / num_rows.max(1) as f32;
                        let factor = (norm_x * 0.75 + norm_y * 0.25).clamp(0.0, 1.0);
                        let (r, g, b) = multi_stop_gradient(stops, factor);

                        buf.get_mut(cell_x, current_y)
                            .set_char(c)
                            .set_fg(Color::Rgb(r, g, b))
                            .set_style(Style::default().add_modifier(Modifier::BOLD));
                    }
                }
                current_y += 1;
            }
        }

        // 2. Render Subtitle & Tagline
        if self.config.show_tagline && current_y < area.bottom() {
            let tagline = self
                .info
                .custom_tagline
                .as_deref()
                .unwrap_or(DEFAULT_SUBTITLE);

            let version_str = self.info.version_display();
            let text = format!("✦ {} {} ✦", tagline, version_str);

            let x_start = if self.config.center_logo {
                area.x + area.width.saturating_sub(text.len() as u16) / 2
            } else {
                area.x + self.config.left_margin as u16
            };

            buf.set_string(
                x_start,
                current_y,
                &text,
                Style::default().fg(theme.primary),
            );
            current_y += 1;
        }

        // 3. Render Metadata line
        if self.config.show_metadata && current_y < area.bottom() {
            let meta_text = format!(
                "Provider: {}  Model: {}  Advisors: {}",
                self.info.provider_display(),
                self.info.model,
                self.info.advisors_display()
            );

            let x_start = area.x + self.config.left_margin as u16;
            buf.set_string(
                x_start,
                current_y,
                &meta_text,
                Style::default().fg(theme.muted),
            );
            current_y += 1;
        }

        // 4. Render Tips
        if self.config.show_tips && current_y < area.bottom() {
            let tip = self
                .config
                .tip_override
                .as_deref()
                .unwrap_or("Type prompt, /help for commands, or /model to switch.");

            let tip_text = format!("Tip: {}", tip);
            let x_start = area.x + self.config.left_margin as u16;
            buf.set_string(
                x_start,
                current_y,
                &tip_text,
                Style::default().fg(theme.accent),
            );
        }
    }
}

// ============================================================================
// 8. Public Convenience Functions
// ============================================================================

/// Render default ANSI startup banner for given `BannerInfo`.
pub fn render_banner_ansi(info: &BannerInfo) -> String {
    let config = BannerConfig::default();
    render_banner(info, &config)
}

/// Render a compact ANSI banner suitable for mobile / Termux.
pub fn render_compact_banner_ansi(info: &BannerInfo) -> String {
    let config = BannerConfig::compact();
    render_banner(info, &config)
}

/// Render a minimal 2-line ANSI header banner.
pub fn render_minimal_banner_ansi(info: &BannerInfo) -> String {
    let config = BannerConfig::minimal();
    render_banner(info, &config)
}

/// Render a single-line ANSI status pill banner.
pub fn render_oneline_banner_ansi(info: &BannerInfo) -> String {
    let mut config = BannerConfig::minimal();
    config.style = BannerStyle::OneLine;
    render_banner(info, &config)
}

/// Render banner to any writer implementing `std::io::Write`.
pub fn render_banner_to<W: Write>(
    w: &mut W,
    info: &BannerInfo,
    config: &BannerConfig,
) -> std::io::Result<()> {
    let rendered = render_banner(info, config);
    w.write_all(rendered.as_bytes())?;
    w.flush()
}

/// Print the stylized Fusion Code AI startup banner to standard output.
pub fn print_startup_banner(config: &Config) {
    let info = BannerInfo::from_config(config);
    let banner_config = BannerConfig::default();
    let output = render_banner(&info, &banner_config);
    print!("{}", output);
    let _ = stdout().flush();
}

/// Drop-in replacement for REPL banner output.
pub fn print_banner(config: &Config) {
    print_startup_banner(config);
}

/// Render a startup banner string from the application [`Config`].
///
/// Version is read from `CARGO_PKG_VERSION` at compile time.
/// Color capability is auto-detected from the terminal environment.
/// Use [`render_banner`] directly with a custom [`BannerConfig`] for finer control.
pub fn render_banner_for_config(config: &Config) -> String {
    let info = BannerInfo::from_config(config);
    let banner_config = BannerConfig::default();
    render_banner(&info, &banner_config)
}

// ============================================================================
// 9. Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_banner_info_defaults() {
        let info = BannerInfo::default();
        assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
        assert_eq!(info.provider, "deepseek");
        assert_eq!(info.model, "deepseek-chat");
        assert!(info.advisors_enabled);
        assert_eq!(info.provider_display(), "DeepSeek");
        assert_eq!(info.advisors_display(), "on");
    }

    #[test]
    fn test_banner_info_builder() {
        let info = BannerInfo::builder()
            .version("1.0.0")
            .provider("anthropic")
            .model("claude-3-7-sonnet")
            .advisors_enabled(true)
            .advisor_model(Some("deepseek-reasoner".to_string()))
            .git_branch(Some("feature/banner".to_string()))
            .add_badge("Status", "Active")
            .build();

        assert_eq!(info.version_display(), "v1.0.0");
        assert_eq!(info.provider_display(), "Anthropic");
        assert_eq!(info.advisors_display(), "on (deepseek-reasoner)");
        assert_eq!(info.git_branch.as_deref(), Some("feature/banner"));
        assert_eq!(info.custom_badges.len(), 1);
    }

    #[test]
    fn test_banner_styles_art_width() {
        assert!(BannerStyle::Cyber.art_width() > 30);
        assert!(BannerStyle::Sleek.art_width() > 30);
        assert!(BannerStyle::Slant.art_width() > 30);
        assert!(BannerStyle::Standard.art_width() > 20);
        assert!(BannerStyle::Compact.art_width() > 10);
        assert_eq!(BannerStyle::Minimal.art_width(), 0);
    }

    #[test]
    fn test_banner_style_resolution_for_width() {
        assert_eq!(BannerStyle::Auto.resolve_for_width(100), BannerStyle::Sleek);
        assert_eq!(
            BannerStyle::Auto.resolve_for_width(65),
            BannerStyle::Standard
        );
        assert_eq!(
            BannerStyle::Auto.resolve_for_width(50),
            BannerStyle::Compact
        );
        assert_eq!(
            BannerStyle::Auto.resolve_for_width(35),
            BannerStyle::Minimal
        );
        assert_eq!(
            BannerStyle::Auto.resolve_for_width(20),
            BannerStyle::OneLine
        );
        // Fixed styles should not change
        assert_eq!(BannerStyle::Sleek.resolve_for_width(20), BannerStyle::Sleek);
    }

    #[test]
    fn test_gradient_interpolation() {
        let c1 = (0, 0, 0);
        let c2 = (100, 200, 50);

        assert_eq!(interpolate_rgb(c1, c2, 0.0), (0, 0, 0));
        assert_eq!(interpolate_rgb(c1, c2, 1.0), (100, 200, 50));
        assert_eq!(interpolate_rgb(c1, c2, 0.5), (50, 100, 25));
    }

    #[test]
    fn test_multi_stop_gradient() {
        let stops = GradientPreset::TokyoNight.stops();
        let first = stops[0].1;
        let last = stops[stops.len() - 1].1;

        assert_eq!(multi_stop_gradient(stops, 0.0), first);
        assert_eq!(multi_stop_gradient(stops, 1.0), last);

        let mid = multi_stop_gradient(stops, 0.5);
        assert_eq!(mid, stops[1].1);
    }

    #[test]
    fn test_apply_horizontal_gradient() {
        let text = "FUSION";
        let stops = GradientPreset::TokyoNight.stops();
        let colored = apply_horizontal_gradient(text, stops);

        assert!(colored.contains("\x1b[38;2;"));
        assert!(colored.contains("\x1b[0m"));
        assert_eq!(strip_ansi(&colored), "FUSION");
    }

    #[test]
    fn test_apply_diagonal_gradient() {
        let lines = &["FUSION", "CODEAI"];
        let stops = GradientPreset::CyberNeon.stops();
        let colored_lines = apply_diagonal_gradient(lines, stops);

        assert_eq!(colored_lines.len(), 2);
        assert_eq!(strip_ansi(&colored_lines[0]), "FUSION");
        assert_eq!(strip_ansi(&colored_lines[1]), "CODEAI");
    }

    #[test]
    fn test_render_banner_monochrome() {
        let info = BannerInfo::new("0.3.0", "deepseek", "deepseek-chat");
        let config = BannerConfig::builder()
            .style(BannerStyle::Standard)
            .color_mode(BannerColorMode::Monochrome)
            .width(80)
            .build();

        let rendered = render_banner(&info, &config);
        assert!(!rendered.contains("\x1b["));
        assert!(rendered.contains("Provider: DeepSeek"));
        assert!(rendered.contains("Model: deepseek-chat"));
        assert!(rendered.contains("PURE-RUST AI CODING ASSISTANT"));
    }

    #[test]
    fn test_render_banner_all_styles() {
        let info = BannerInfo::default();
        let styles = [
            BannerStyle::Cyber,
            BannerStyle::Sleek,
            BannerStyle::Slant,
            BannerStyle::Standard,
            BannerStyle::Compact,
            BannerStyle::Minimal,
            BannerStyle::OneLine,
        ];

        for style in styles {
            let config = BannerConfig::builder()
                .style(style)
                .color_mode(BannerColorMode::Monochrome)
                .width(80)
                .build();

            let rendered = render_banner(&info, &config);
            assert!(
                rendered.contains("Fusion")
                    || rendered.contains("Provider:")
                    || rendered.contains("DeepSeek")
            );
        }
    }

    #[test]
    fn test_render_banner_box_borders() {
        let info = BannerInfo::default();
        let borders = [
            BannerBoxBorder::Rounded,
            BannerBoxBorder::Single,
            BannerBoxBorder::Double,
            BannerBoxBorder::Heavy,
            BannerBoxBorder::Ascii,
            BannerBoxBorder::HorizontalRules,
        ];

        for border in borders {
            let config = BannerConfig::builder()
                .style(BannerStyle::Minimal)
                .box_border(border)
                .color_mode(BannerColorMode::Monochrome)
                .width(60)
                .build();

            let rendered = render_banner(&info, &config);
            let (tl, _, bl, _, _, _) = border.chars();
            assert!(rendered.contains(tl));
            assert!(rendered.contains(bl));
        }
    }

    #[test]
    fn test_render_banner_to_writer() {
        let info = BannerInfo::default();
        let config = BannerConfig::minimal();
        let mut buffer = Vec::new();

        render_banner_to(&mut buffer, &info, &config).unwrap();
        let output = String::from_utf8(buffer).unwrap();
        assert!(output.contains("Fusion"));
    }

    #[test]
    fn test_ratatui_banner_widget_render() {
        let info = BannerInfo::default();
        let config = BannerConfig::default();
        let widget = BannerWidget::new(&info, &config);

        let area = Rect::new(0, 0, 80, 20);
        let mut buffer = Buffer::empty(area);

        widget.render(area, &mut buffer);

        // Verify buffer has content written
        let mut has_content = false;
        for y in 0..area.height {
            for x in 0..area.width {
                if buffer.get(x, y).symbol() != " " {
                    has_content = true;
                    break;
                }
            }
        }
        assert!(has_content);
    }

    /// Verify no-color (Monochrome) output contains version, model, and provider
    /// without any ANSI escape sequences.
    #[test]
    fn test_render_banner_no_color() {
        let version = env!("CARGO_PKG_VERSION");
        let info = BannerInfo::new(version, "anthropic", "claude-opus-4");
        let config = BannerConfig::builder()
            .style(BannerStyle::Standard)
            .color_mode(BannerColorMode::Monochrome)
            .width(80)
            .build();

        let rendered = render_banner(&info, &config);

        // No ANSI escape sequences present
        assert!(
            !rendered.contains("\x1b["),
            "no-color banner must not contain ANSI escapes"
        );
        // Version appears (with v-prefix)
        assert!(
            rendered.contains(version),
            "no-color banner must contain version"
        );
        // Provider and model appear in metadata rows
        assert!(
            rendered.contains("Anthropic"),
            "no-color banner must contain provider display name"
        );
        assert!(
            rendered.contains("claude-opus-4"),
            "no-color banner must contain model name"
        );
    }
}
