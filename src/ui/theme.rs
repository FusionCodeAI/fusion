//! Adaptive terminal color themes for Fusion.
//!
//! Provides Tokyo Night, Monokai, Dracula, and High Contrast themes
//! with automatic terminal background detection (dark vs light)
//! and a clean theme abstraction for inline Ratatui rendering and terminal output.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Wrap},
    Frame,
};
use serde::{Deserialize, Serialize};

use crate::ui::inline::StatusInfo;

/// Terminal background brightness mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BackgroundMode {
    /// Dark background (default for most terminals)
    Dark,
    /// Light background (e.g. paper white or solarized light)
    Light,
}

impl Default for BackgroundMode {
    fn default() -> Self {
        Self::Dark
    }
}

impl BackgroundMode {
    /// Returns true if the background is dark.
    pub fn is_dark(&self) -> bool {
        matches!(self, Self::Dark)
    }

    /// Returns true if the background is light.
    pub fn is_light(&self) -> bool {
        matches!(self, Self::Light)
    }

    /// Auto-detect the terminal background brightness mode.
    ///
    /// Checks the following indicators in order:
    /// 1. `FUSION_THEME_MODE` or `TERMINAL_BACKGROUND` or `TERM_BACKGROUND`
    /// 2. `COLORFGBG` environment variable (set by xterm, rxvt, konsole, etc.)
    /// 3. macOS Terminal window background heuristics via environment
    /// 4. Fallback: defaults to `Dark`
    pub fn detect() -> Self {
        // 1. Explicit override variables
        for var in &[
            "FUSION_THEME_MODE",
            "FUSION_BACKGROUND",
            "TERMINAL_BACKGROUND",
            "TERM_BACKGROUND",
        ] {
            if let Ok(val) = std::env::var(var) {
                let lower = val.to_ascii_lowercase();
                if lower.contains("light") || lower.contains("white") {
                    return Self::Light;
                } else if lower.contains("dark") || lower.contains("black") {
                    return Self::Dark;
                }
            }
        }

        // 2. Parse COLORFGBG (e.g. "15;0" or "0;15" or "default;default")
        // Standard syntax is "fg;bg" or "fg;extra;bg". The last integer is the background color code.
        // Color codes 0..=6 and 8 are dark; 7 and 9..=15 are light.
        if let Ok(colorfgbg) = std::env::var("COLORFGBG") {
            if let Some(bg_str) = colorfgbg.rsplit(';').next() {
                if let Ok(bg_code) = bg_str.trim().parse::<u8>() {
                    return match bg_code {
                        0..=6 | 8 => Self::Dark,
                        7 | 9..=15 => Self::Light,
                        _ => {
                            // 256 color palette heuristic:
                            // 16-231 is 6x6x6 color cube, 232-255 is grayscale ramp
                            if (232..=243).contains(&bg_code) {
                                Self::Dark
                            } else if (244..=255).contains(&bg_code) {
                                Self::Light
                            } else {
                                Self::Dark
                            }
                        }
                    };
                }
            }
        }

        // 3. MacOS Terminal profile heuristic if available
        if let Ok(term_prog) = std::env::var("TERM_PROGRAM") {
            if term_prog == "Apple_Terminal" {
                if let Ok(term_profile) = std::env::var("Apple_Terminal_Profile") {
                    let lower = term_profile.to_ascii_lowercase();
                    if lower.contains("basic")
                        || lower.contains("white")
                        || lower.contains("light")
                        || lower.contains("silver")
                    {
                        return Self::Light;
                    }
                }
            }
        }

        // Default to Dark
        Self::Dark
    }
}

/// Available color theme families in Fusion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemeKind {
    /// Tokyo Night theme (balanced deep blues and neon accents)
    TokyoNight,
    /// Monokai theme (warm charcoal with vibrant lime/yellow/magenta)
    Monokai,
    /// Dracula theme (gothic dark with pastel purples and cyans)
    Dracula,
    /// High Contrast theme (maximum readability and accessibility)
    HighContrast,
    /// Adaptive theme that selects the best theme based on background
    Adaptive,
}

impl Default for ThemeKind {
    fn default() -> Self {
        Self::TokyoNight
    }
}

impl ThemeKind {
    /// Parse theme kind from string (case-insensitive, kebab/snake allowed).
    pub fn from_name(name: &str) -> Option<Self> {
        let normalized = name
            .trim()
            .to_ascii_lowercase()
            .replace(['-', '_', ' '], "");
        match normalized.as_str() {
            "tokyonight" | "tokyo" | "night" => Some(Self::TokyoNight),
            "monokai" => Some(Self::Monokai),
            "dracula" => Some(Self::Dracula),
            "highcontrast" | "contrast" | "hc" | "accessible" => Some(Self::HighContrast),
            "adaptive" | "auto" | "default" => Some(Self::Adaptive),
            _ => None,
        }
    }

    /// Canonical theme name string.
    pub fn name(&self) -> &'static str {
        match self {
            Self::TokyoNight => "Tokyo Night",
            Self::Monokai => "Monokai",
            Self::Dracula => "Dracula",
            Self::HighContrast => "High Contrast",
            Self::Adaptive => "Adaptive",
        }
    }

    /// Identifier used in configs and CLI flags.
    pub fn id(&self) -> &'static str {
        match self {
            Self::TokyoNight => "tokyo-night",
            Self::Monokai => "monokai",
            Self::Dracula => "dracula",
            Self::HighContrast => "high-contrast",
            Self::Adaptive => "adaptive",
        }
    }

    /// List all available theme kinds.
    pub fn all() -> &'static [Self] {
        &[
            Self::TokyoNight,
            Self::Monokai,
            Self::Dracula,
            Self::HighContrast,
            Self::Adaptive,
        ]
    }
}

/// Complete color palette and styling definitions for Fusion UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Theme {
    /// Theme display name
    pub name: String,
    /// Theme kind
    pub kind: ThemeKind,
    /// Background mode (Dark or Light)
    pub mode: BackgroundMode,

    // Base palette
    /// Main brand / primary accent color
    pub primary: Color,
    /// Secondary accent color
    pub secondary: Color,
    /// Third accent / highlight color
    pub accent: Color,
    /// Default foreground text color
    pub foreground: Color,
    /// Muted / dimmed text color
    pub muted: Color,
    /// Optional explicit background color
    pub background: Option<Color>,
    /// Panel and card border color
    pub border: Color,
    /// Focused / active border color
    pub border_focused: Color,

    // Status / semantic colors
    /// Success / OK / Approved color
    pub success: Color,
    /// Warning / Critique / Notice color
    pub warning: Color,
    /// Error / Failure / Danger color
    pub error: Color,
    /// Information / Hint color
    pub info: Color,

    // UI-specific element colors
    /// LLM provider badge color
    pub provider: Color,
    /// LLM model name color
    pub model: Color,
    /// Agent subagent badge color
    pub agent: Color,
    /// Advisor critique badge color
    pub advisor: Color,
    /// Running activity status color
    pub status: Color,
    /// Highlighted selection background
    pub selection: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Self::auto()
    }
}

impl Theme {
    /// Auto-detect background mode and create the best matching theme.
    ///
    /// Respects the `FUSION_THEME` environment variable if set.
    pub fn auto() -> Self {
        let mode = BackgroundMode::detect();
        if let Ok(theme_env) = std::env::var("FUSION_THEME") {
            if let Some(kind) = ThemeKind::from_name(&theme_env) {
                return Self::for_kind(kind, mode);
            }
        }
        Self::adaptive(mode)
    }

    /// Create an adaptive theme suited for the given background mode.
    pub fn adaptive(mode: BackgroundMode) -> Self {
        match mode {
            BackgroundMode::Dark => Self::tokyo_night(),
            BackgroundMode::Light => Self::tokyo_night_day(),
        }
    }

    /// Resolve a theme by name with auto background detection.
    pub fn by_name(name: &str) -> Option<Self> {
        let kind = ThemeKind::from_name(name)?;
        let mode = BackgroundMode::detect();
        Some(Self::for_kind(kind, mode))
    }

    /// Create a theme by kind and explicit background mode.
    pub fn for_kind(kind: ThemeKind, mode: BackgroundMode) -> Self {
        match (kind, mode) {
            (ThemeKind::TokyoNight, BackgroundMode::Dark) => Self::tokyo_night(),
            (ThemeKind::TokyoNight, BackgroundMode::Light) => Self::tokyo_night_day(),

            (ThemeKind::Monokai, BackgroundMode::Dark) => Self::monokai(),
            (ThemeKind::Monokai, BackgroundMode::Light) => Self::monokai_light(),

            (ThemeKind::Dracula, BackgroundMode::Dark) => Self::dracula(),
            (ThemeKind::Dracula, BackgroundMode::Light) => Self::dracula_light(),

            (ThemeKind::HighContrast, BackgroundMode::Dark) => Self::high_contrast_dark(),
            (ThemeKind::HighContrast, BackgroundMode::Light) => Self::high_contrast_light(),

            (ThemeKind::Adaptive, mode) => Self::adaptive(mode),
        }
    }

    // --- Tokyo Night Themes ---

    /// Tokyo Night (Dark) - Deep indigo background with cyan and magenta accents.
    pub fn tokyo_night() -> Self {
        Self {
            name: "Tokyo Night".to_string(),
            kind: ThemeKind::TokyoNight,
            mode: BackgroundMode::Dark,

            primary: Color::Rgb(125, 207, 255),    // Cyan #7dcfff
            secondary: Color::Rgb(122, 162, 247),  // Blue #7aa2f7
            accent: Color::Rgb(187, 154, 247),     // Purple #bb9af7
            foreground: Color::Rgb(192, 202, 245), // Text #c0caf5
            muted: Color::Rgb(86, 95, 137),        // Comment #565f89
            background: Some(Color::Rgb(26, 27, 38)), // Bg #1a1b26
            border: Color::Rgb(65, 72, 104),       // Border #414868
            border_focused: Color::Rgb(125, 207, 255),

            success: Color::Rgb(158, 206, 106), // Green #9ece6a
            warning: Color::Rgb(224, 175, 104), // Yellow/Orange #e0af68
            error: Color::Rgb(247, 118, 142),   // Red #f7768e
            info: Color::Rgb(125, 207, 255),    // Cyan #7dcfff

            provider: Color::Rgb(125, 207, 255), // Cyan
            model: Color::Rgb(192, 202, 245),    // Light White
            agent: Color::Rgb(187, 154, 247),    // Magenta
            advisor: Color::Rgb(224, 175, 104),  // Yellow
            status: Color::Rgb(158, 206, 106),   // Green
            selection: Color::Rgb(40, 52, 87),   // #283457
        }
    }

    /// Tokyo Night Day (Light) - Clean light background with high-contrast Tokyo tones.
    pub fn tokyo_night_day() -> Self {
        Self {
            name: "Tokyo Night Day".to_string(),
            kind: ThemeKind::TokyoNight,
            mode: BackgroundMode::Light,

            primary: Color::Rgb(55, 96, 191),   // Deep Blue #3760bf
            secondary: Color::Rgb(15, 75, 160), // Darker Blue #0f4ba0
            accent: Color::Rgb(152, 84, 194),   // Deep Purple #9854c2
            foreground: Color::Rgb(52, 59, 88), // Dark Charcoal #343b58
            muted: Color::Rgb(137, 142, 164),   // Muted #898ea4
            background: Some(Color::Rgb(225, 226, 231)), // Light Gray #e1e2e7
            border: Color::Rgb(180, 185, 206),  // Light Border #b4b9ce
            border_focused: Color::Rgb(55, 96, 191),

            success: Color::Rgb(88, 117, 57),  // Olive Green #587539
            warning: Color::Rgb(140, 108, 62), // Ochre/Amber #8c6c3e
            error: Color::Rgb(245, 42, 101),   // Crimson #f52a65
            info: Color::Rgb(0, 107, 184),     // Deep Cyan #006bb8

            provider: Color::Rgb(55, 96, 191), // Deep Blue
            model: Color::Rgb(52, 59, 88),     // Dark text
            agent: Color::Rgb(152, 84, 194),   // Deep Purple
            advisor: Color::Rgb(140, 108, 62), // Ochre
            status: Color::Rgb(88, 117, 57),   // Olive Green
            selection: Color::Rgb(200, 210, 230),
        }
    }

    // --- Monokai Themes ---

    /// Monokai (Dark) - Classic warm charcoal with vibrant neon green, yellow, and magenta.
    pub fn monokai() -> Self {
        Self {
            name: "Monokai".to_string(),
            kind: ThemeKind::Monokai,
            mode: BackgroundMode::Dark,

            primary: Color::Rgb(102, 217, 239),    // Cyan #66d9ef
            secondary: Color::Rgb(174, 129, 255),  // Purple #ae81ff
            accent: Color::Rgb(253, 151, 31),      // Orange #fd971f
            foreground: Color::Rgb(248, 248, 242), // White/Off-white #f8f8f2
            muted: Color::Rgb(117, 113, 94),       // Dim Stone #75715e
            background: Some(Color::Rgb(39, 40, 34)), // Charcoal #272822
            border: Color::Rgb(73, 72, 62),        // Dark Stone #49483e
            border_focused: Color::Rgb(102, 217, 239),

            success: Color::Rgb(166, 226, 46),  // Lime #a6e22e
            warning: Color::Rgb(230, 219, 116), // Yellow #e6db74
            error: Color::Rgb(249, 38, 114),    // Hot Pink #f92672
            info: Color::Rgb(102, 217, 239),    // Cyan #66d9ef

            provider: Color::Rgb(102, 217, 239), // Cyan
            model: Color::Rgb(248, 248, 242),    // Bright White
            agent: Color::Rgb(174, 129, 255),    // Purple
            advisor: Color::Rgb(230, 219, 116),  // Yellow
            status: Color::Rgb(166, 226, 46),    // Lime Green
            selection: Color::Rgb(60, 60, 50),
        }
    }

    /// Monokai Light - Warm light background with rich Monokai-derived tones.
    pub fn monokai_light() -> Self {
        Self {
            name: "Monokai Light".to_string(),
            kind: ThemeKind::Monokai,
            mode: BackgroundMode::Light,

            primary: Color::Rgb(0, 130, 160),            // Deep Teal
            secondary: Color::Rgb(120, 70, 190),         // Violet
            accent: Color::Rgb(200, 100, 0),             // Warm Amber
            foreground: Color::Rgb(40, 40, 35),          // Dark Charcoal
            muted: Color::Rgb(130, 130, 120),            // Warm Gray
            background: Some(Color::Rgb(250, 249, 246)), // Warm White
            border: Color::Rgb(205, 205, 195),           // Warm Border
            border_focused: Color::Rgb(0, 130, 160),

            success: Color::Rgb(70, 140, 20),  // Deep Lime
            warning: Color::Rgb(180, 140, 20), // Dark Gold
            error: Color::Rgb(200, 20, 80),    // Deep Pink/Red
            info: Color::Rgb(0, 130, 160),     // Deep Teal

            provider: Color::Rgb(0, 130, 160), // Teal
            model: Color::Rgb(40, 40, 35),     // Charcoal
            agent: Color::Rgb(120, 70, 190),   // Violet
            advisor: Color::Rgb(180, 140, 20), // Dark Gold
            status: Color::Rgb(70, 140, 20),   // Deep Lime
            selection: Color::Rgb(230, 230, 220),
        }
    }

    // --- Dracula Themes ---

    /// Dracula (Dark) - Gothic dark theme with soft pastels, purple, pink, and cyan.
    pub fn dracula() -> Self {
        Self {
            name: "Dracula".to_string(),
            kind: ThemeKind::Dracula,
            mode: BackgroundMode::Dark,

            primary: Color::Rgb(139, 233, 253),    // Cyan #8be9fd
            secondary: Color::Rgb(189, 147, 249),  // Purple #bd93f9
            accent: Color::Rgb(255, 121, 198),     // Pink #ff79c6
            foreground: Color::Rgb(248, 248, 242), // White #f8f8f2
            muted: Color::Rgb(98, 114, 164),       // Comment Blue #6272a4
            background: Some(Color::Rgb(40, 42, 54)), // Background #282a36
            border: Color::Rgb(68, 71, 90),        // Current line #44475a
            border_focused: Color::Rgb(189, 147, 249),

            success: Color::Rgb(80, 250, 123),  // Green #50fa7b
            warning: Color::Rgb(241, 250, 140), // Yellow #f1fa8c
            error: Color::Rgb(255, 85, 85),     // Red #ff5555
            info: Color::Rgb(139, 233, 253),    // Cyan #8be9fd

            provider: Color::Rgb(139, 233, 253), // Cyan
            model: Color::Rgb(248, 248, 242),    // White
            agent: Color::Rgb(255, 121, 198),    // Pink
            advisor: Color::Rgb(241, 250, 140),  // Yellow
            status: Color::Rgb(80, 250, 123),    // Green
            selection: Color::Rgb(68, 71, 90),   // Current line
        }
    }

    /// Dracula Light - Light pastel theme with Dracula aesthetic adapted for bright backgrounds.
    pub fn dracula_light() -> Self {
        Self {
            name: "Dracula Light".to_string(),
            kind: ThemeKind::Dracula,
            mode: BackgroundMode::Light,

            primary: Color::Rgb(0, 120, 145),            // Dark Cyan
            secondary: Color::Rgb(110, 75, 180),         // Violet
            accent: Color::Rgb(190, 40, 120),            // Magenta
            foreground: Color::Rgb(40, 42, 54),          // Dark Indigo
            muted: Color::Rgb(120, 130, 160),            // Slate
            background: Some(Color::Rgb(248, 248, 242)), // Cream #f8f8f2
            border: Color::Rgb(190, 195, 210),           // Soft Border
            border_focused: Color::Rgb(110, 75, 180),

            success: Color::Rgb(40, 150, 70),  // Forest Green
            warning: Color::Rgb(160, 140, 20), // Dark Gold
            error: Color::Rgb(210, 40, 40),    // Deep Red
            info: Color::Rgb(0, 120, 145),     // Dark Cyan

            provider: Color::Rgb(0, 120, 145), // Dark Cyan
            model: Color::Rgb(40, 42, 54),     // Indigo
            agent: Color::Rgb(190, 40, 120),   // Magenta
            advisor: Color::Rgb(160, 140, 20), // Dark Gold
            status: Color::Rgb(40, 150, 70),   // Green
            selection: Color::Rgb(225, 225, 235),
        }
    }

    // --- High Contrast Themes ---

    /// High Contrast (Dark) - Pitch black background with vivid, maximum-contrast colors.
    pub fn high_contrast_dark() -> Self {
        Self {
            name: "High Contrast (Dark)".to_string(),
            kind: ThemeKind::HighContrast,
            mode: BackgroundMode::Dark,

            primary: Color::Cyan,           // Pure Cyan
            secondary: Color::LightBlue,    // Light Blue
            accent: Color::Magenta,         // Pure Magenta
            foreground: Color::White,       // Pure White
            muted: Color::Gray,             // Clear Gray
            background: Some(Color::Black), // True Black
            border: Color::White,           // Solid White Border
            border_focused: Color::Yellow,

            success: Color::LightGreen, // Bright Green
            warning: Color::Yellow,     // Bright Yellow
            error: Color::LightRed,     // Bright Red
            info: Color::Cyan,          // Cyan

            provider: Color::Cyan,     // Cyan
            model: Color::White,       // White
            agent: Color::Magenta,     // Magenta
            advisor: Color::Yellow,    // Yellow
            status: Color::LightGreen, // Green
            selection: Color::DarkGray,
        }
    }

    /// High Contrast (Light) - Crisp white background with dense, high-contrast dark tones.
    pub fn high_contrast_light() -> Self {
        Self {
            name: "High Contrast (Light)".to_string(),
            kind: ThemeKind::HighContrast,
            mode: BackgroundMode::Light,

            primary: Color::Rgb(0, 50, 150),   // Bold Navy
            secondary: Color::Rgb(80, 0, 120), // Deep Purple
            accent: Color::Rgb(160, 40, 0),    // Deep Rust
            foreground: Color::Black,          // True Black
            muted: Color::DarkGray,            // Dark Gray
            background: Some(Color::White),    // Pure White
            border: Color::Black,              // Solid Black Border
            border_focused: Color::Rgb(0, 50, 150),

            success: Color::Rgb(0, 120, 0),  // Crisp Dark Green
            warning: Color::Rgb(140, 80, 0), // Crisp Dark Amber
            error: Color::Rgb(180, 0, 0),    // Crisp Dark Red
            info: Color::Rgb(0, 50, 150),    // Bold Navy

            provider: Color::Rgb(0, 50, 150), // Bold Navy
            model: Color::Black,              // Black
            agent: Color::Rgb(80, 0, 120),    // Deep Purple
            advisor: Color::Rgb(140, 80, 0),  // Dark Amber
            status: Color::Rgb(0, 120, 0),    // Dark Green
            selection: Color::Gray,
        }
    }

    /// High contrast theme respecting the specified background mode.
    pub fn high_contrast(mode: BackgroundMode) -> Self {
        match mode {
            BackgroundMode::Dark => Self::high_contrast_dark(),
            BackgroundMode::Light => Self::high_contrast_light(),
        }
    }

    // --- Style Helpers ---

    /// Style for provider badge (bold).
    pub fn provider_style(&self) -> Style {
        Style::default()
            .fg(self.provider)
            .add_modifier(Modifier::BOLD)
    }

    /// Style for model name (bold).
    pub fn model_style(&self) -> Style {
        Style::default().fg(self.model).add_modifier(Modifier::BOLD)
    }

    /// Style for active agent badge (bold).
    pub fn agent_style(&self) -> Style {
        Style::default().fg(self.agent).add_modifier(Modifier::BOLD)
    }

    /// Style for active advisor badge (bold).
    pub fn advisor_style(&self) -> Style {
        Style::default()
            .fg(self.advisor)
            .add_modifier(Modifier::BOLD)
    }

    /// Style for running activity status.
    pub fn status_style(&self) -> Style {
        Style::default().fg(self.status)
    }

    /// Style for dimmed / token count text.
    pub fn muted_style(&self) -> Style {
        Style::default().fg(self.muted)
    }

    /// Style for borders.
    pub fn border_style(&self) -> Style {
        Style::default().fg(self.border)
    }

    /// Style for success (e.g. approved critiques).
    pub fn success_style(&self) -> Style {
        Style::default()
            .fg(self.success)
            .add_modifier(Modifier::BOLD)
    }

    /// Style for warnings (e.g. active critiques).
    pub fn warning_style(&self) -> Style {
        Style::default()
            .fg(self.warning)
            .add_modifier(Modifier::BOLD)
    }

    /// Style for errors.
    pub fn error_style(&self) -> Style {
        Style::default().fg(self.error).add_modifier(Modifier::BOLD)
    }

    /// Style for informational items.
    pub fn info_style(&self) -> Style {
        Style::default().fg(self.info)
    }

    // --- Inline Rendering Abstractions ---

    /// Build styled status spans for the inline status bar.
    pub fn build_status_spans<'a>(&self, info: &'a StatusInfo, is_compact: bool) -> Vec<Span<'a>> {
        let mut spans = Vec::new();

        // ✦ Sparkle icon & Provider
        spans.push(Span::styled(" ✦ ", self.provider_style()));
        spans.push(Span::styled(&info.provider, self.provider_style()));
        spans.push(Span::raw("/"));
        spans.push(Span::styled(&info.model, self.model_style()));

        // Active agent badge
        if let Some(agent) = &info.active_agent {
            spans.push(Span::raw(" │ "));
            let tag = if is_compact {
                format!("A:{}", agent)
            } else {
                format!("Agent: {}", agent)
            };
            spans.push(Span::styled(tag, self.agent_style()));
        }

        // Active advisor badge
        if let Some(advisor) = &info.active_advisor {
            spans.push(Span::raw(" │ "));
            let tag = if is_compact {
                format!("Adv:{}", advisor)
            } else {
                format!("Advisor: {}", advisor)
            };
            spans.push(Span::styled(tag, self.advisor_style()));
        }

        // Status message
        if !info.status.is_empty() {
            spans.push(Span::raw(" │ "));
            spans.push(Span::styled(&info.status, self.status_style()));
        }

        // Token usage if space permits
        if !is_compact {
            if let Some(tokens) = &info.token_usage {
                spans.push(Span::raw(" │ "));
                spans.push(Span::styled(tokens, self.muted_style()));
            }
        }

        spans
    }

    /// Render a themed status bar into the designated terminal frame area.
    pub fn render_status_bar(&self, f: &mut Frame, area: Rect, info: &StatusInfo) {
        let cols = area.width;
        let is_compact = cols < 60; // Mobile / Termux portrait friendly

        let spans = self.build_status_spans(info, is_compact);
        let paragraph = Paragraph::new(Line::from(spans))
            .block(
                Block::default()
                    .borders(Borders::TOP)
                    .border_type(BorderType::Plain)
                    .border_style(self.border_style()),
            )
            .wrap(Wrap { trim: true });

        f.render_widget(paragraph, area);
    }

    /// Render an informational card with themed borders and title.
    pub fn render_card(
        &self,
        f: &mut Frame,
        area: Rect,
        title: &str,
        content: &str,
        border_color: Option<Color>,
    ) {
        let border_fg = border_color.unwrap_or(self.primary);
        let block = Block::default()
            .title(format!(" {} ", title))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(border_fg));

        let paragraph = Paragraph::new(content)
            .block(block)
            .wrap(Wrap { trim: true });

        f.render_widget(paragraph, area);
    }

    /// Render an advisor critique card inside an inline Frame using active theme semantics.
    pub fn render_critique_card(
        &self,
        f: &mut Frame,
        area: Rect,
        advisor_name: &str,
        approved: bool,
        critique: &str,
    ) {
        let (border_color, status_icon, status_text) = if approved {
            (self.success, "✓", "APPROVED")
        } else {
            (self.warning, "!", "CRITIQUE")
        };

        let title = format!(
            " [Advisor: {}] {} {} ",
            advisor_name, status_icon, status_text
        );

        let block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(border_color));

        let paragraph = Paragraph::new(critique)
            .block(block)
            .wrap(Wrap { trim: true });

        f.render_widget(paragraph, area);
    }

    /// Format an ANSI prompt symbol string matching this theme.
    pub fn prompt_symbol_ansi(&self) -> String {
        // High contrast uses bright ANSI; others use truecolor or ANSI
        match self.mode {
            BackgroundMode::Dark => "\x1b[1;36m❯\x1b[0m ".to_string(),
            BackgroundMode::Light => "\x1b[1;34m❯\x1b[0m ".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn test_background_mode_detection_defaults() {
        // Without special env vars, defaults to dark
        let mode = BackgroundMode::Dark;
        assert!(mode.is_dark());
        assert!(!mode.is_light());

        let light = BackgroundMode::Light;
        assert!(light.is_light());
        assert!(!light.is_dark());
    }

    #[test]
    fn test_theme_kind_parsing() {
        assert_eq!(
            ThemeKind::from_name("tokyo-night"),
            Some(ThemeKind::TokyoNight)
        );
        assert_eq!(
            ThemeKind::from_name("tokyonight"),
            Some(ThemeKind::TokyoNight)
        );
        assert_eq!(
            ThemeKind::from_name("TOKYO_NIGHT"),
            Some(ThemeKind::TokyoNight)
        );
        assert_eq!(ThemeKind::from_name("monokai"), Some(ThemeKind::Monokai));
        assert_eq!(ThemeKind::from_name("dracula"), Some(ThemeKind::Dracula));
        assert_eq!(
            ThemeKind::from_name("high-contrast"),
            Some(ThemeKind::HighContrast)
        );
        assert_eq!(ThemeKind::from_name("hc"), Some(ThemeKind::HighContrast));
        assert_eq!(ThemeKind::from_name("adaptive"), Some(ThemeKind::Adaptive));
        assert_eq!(ThemeKind::from_name("unknown_theme"), None);
    }

    #[test]
    fn test_theme_constructors() {
        let tokyo = Theme::tokyo_night();
        assert_eq!(tokyo.kind, ThemeKind::TokyoNight);
        assert_eq!(tokyo.mode, BackgroundMode::Dark);

        let tokyo_day = Theme::tokyo_night_day();
        assert_eq!(tokyo_day.kind, ThemeKind::TokyoNight);
        assert_eq!(tokyo_day.mode, BackgroundMode::Light);

        let monokai = Theme::monokai();
        assert_eq!(monokai.kind, ThemeKind::Monokai);
        assert_eq!(monokai.mode, BackgroundMode::Dark);

        let dracula = Theme::dracula();
        assert_eq!(dracula.kind, ThemeKind::Dracula);
        assert_eq!(dracula.mode, BackgroundMode::Dark);

        let hc_dark = Theme::high_contrast_dark();
        assert_eq!(hc_dark.kind, ThemeKind::HighContrast);
        assert_eq!(hc_dark.mode, BackgroundMode::Dark);

        let hc_light = Theme::high_contrast_light();
        assert_eq!(hc_light.kind, ThemeKind::HighContrast);
        assert_eq!(hc_light.mode, BackgroundMode::Light);
    }

    #[test]
    fn test_theme_by_name() {
        let theme = Theme::by_name("dracula").expect("dracula theme should exist");
        assert_eq!(theme.kind, ThemeKind::Dracula);

        let hc = Theme::by_name("high-contrast").expect("high-contrast theme should exist");
        assert_eq!(hc.kind, ThemeKind::HighContrast);
    }

    #[test]
    fn test_adaptive_theme() {
        let dark_theme = Theme::adaptive(BackgroundMode::Dark);
        assert_eq!(dark_theme.mode, BackgroundMode::Dark);

        let light_theme = Theme::adaptive(BackgroundMode::Light);
        assert_eq!(light_theme.mode, BackgroundMode::Light);
    }

    #[test]
    fn test_theme_render_status_bar() {
        let backend = TestBackend::new(80, 2);
        let mut terminal = Terminal::new(backend).unwrap();

        let theme = Theme::tokyo_night();
        let info = StatusInfo::new("anthropic", "claude-3-5-sonnet")
            .with_agent("Coder")
            .with_advisor("SecAdvisor")
            .with_status("Indexing codebase")
            .with_tokens("3.4k tokens");

        terminal
            .draw(|f| {
                let area = f.area();
                theme.render_status_bar(f, area, &info);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let text: String = buffer.content().iter().map(|c| c.symbol()).collect();
        assert!(text.contains('✦'));
        assert!(text.contains("anthropic"));
        assert!(text.contains("claude-3-5-sonnet"));
    }

    #[test]
    fn test_theme_render_card() {
        let backend = TestBackend::new(60, 4);
        let mut terminal = Terminal::new(backend).unwrap();

        let theme = Theme::dracula();
        terminal
            .draw(|f| {
                let area = f.area();
                theme.render_card(f, area, "Notice", "Dracula theme active.", None);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let text: String = buffer.content().iter().map(|c| c.symbol()).collect();
        assert!(text.contains("Notice"));
        assert!(text.contains("Dracula theme active."));
    }

    #[test]
    fn test_theme_render_critique_card() {
        let backend = TestBackend::new(70, 5);
        let mut terminal = Terminal::new(backend).unwrap();

        let theme = Theme::monokai();
        terminal
            .draw(|f| {
                let area = f.area();
                theme.render_critique_card(f, area, "LinterAdvisor", true, "All checks passed.");
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let text: String = buffer.content().iter().map(|c| c.symbol()).collect();
        assert!(text.contains('✓'));
        assert!(text.contains("APPROVED"));
        assert!(text.contains("LinterAdvisor"));
    }

    #[test]
    fn test_high_contrast_status_spans() {
        let theme = Theme::high_contrast_dark();
        let info = StatusInfo::new("openai", "gpt-4o").with_status("Ready");

        let spans = theme.build_status_spans(&info, false);
        assert!(!spans.is_empty());
        // Verify high contrast styling
        assert_eq!(theme.provider, Color::Cyan);
        assert_eq!(theme.foreground, Color::White);
    }
}
