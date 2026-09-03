//! Terminal color capability detection and color downsampling for Fusion.
//!
//! Automatically inspects environment variables (`COLORTERM`, `TERM`, `NO_COLOR`,
//! `FORCE_COLOR`, `CLICOLOR_FORCE`, `CLICOLOR`, `TERM_PROGRAM`, etc.) and system
//! properties to detect the terminal's color rendering capabilities:
//! - **No Color / Monochrome**: `NO_COLOR` is set, `TERM=dumb`, or colors disabled.
//! - **16 Colors (ANSI 4-bit)**: Standard terminal ANSI color palette.
//! - **256 Colors (ANSI 8-bit)**: Extended xterm 256-color palette.
//! - **TrueColor (24-bit direct RGB)**: 16.7 million colors (direct RGB).
//!
//! Also provides color quantization/downsampling helpers to gracefully degrade
//! 24-bit RGB colors to 256-color or 16-color palettes when running on constrained
//! terminals.

use ratatui::style::Color;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;

// ---------------------------------------------------------------------------
// ColorCapability Enum
// ---------------------------------------------------------------------------

/// Terminal color rendering capability level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ColorCapability {
    /// No color support (monochrome output, or NO_COLOR requested).
    NoColor = 0,
    /// 16 standard ANSI colors (4-bit color depth).
    Ansi16 = 1,
    /// 256 indexed colors (8-bit color depth).
    Ansi256 = 2,
    /// 24-bit TrueColor / direct RGB (16.7 million colors).
    TrueColor = 3,
}

/// Alias for `ColorCapability` for alternate naming conventions.
pub type ColorSupport = ColorCapability;
pub type ColorLevel = ColorCapability;

impl Default for ColorCapability {
    fn default() -> Self {
        Self::detect()
    }
}

impl fmt::Display for ColorCapability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoColor => write!(f, "NoColor (Monochrome)"),
            Self::Ansi16 => write!(f, "16-Color (ANSI 4-bit)"),
            Self::Ansi256 => write!(f, "256-Color (ANSI 8-bit)"),
            Self::TrueColor => write!(f, "TrueColor (24-bit RGB)"),
        }
    }
}

impl ColorCapability {
    /// Returns `true` if any color output is supported (>= 16 colors).
    pub fn has_color(&self) -> bool {
        *self >= Self::Ansi16
    }

    /// Returns `true` if the terminal supports TrueColor (24-bit direct RGB).
    pub fn is_truecolor(&self) -> bool {
        *self == Self::TrueColor
    }

    /// Returns `true` if the terminal supports at least 256 indexed colors.
    pub fn is_256_color(&self) -> bool {
        *self >= Self::Ansi256
    }

    /// Returns `true` if the terminal only supports basic 16 ANSI colors.
    pub fn is_basic(&self) -> bool {
        *self == Self::Ansi16
    }

    /// Returns `true` if colors are disabled / monochrome.
    pub fn is_no_color(&self) -> bool {
        *self == Self::NoColor
    }

    /// Returns the maximum number of distinct colors supported.
    pub fn max_colors(&self) -> u32 {
        match self {
            Self::NoColor => 0,
            Self::Ansi16 => 16,
            Self::Ansi256 => 256,
            Self::TrueColor => 16_777_216,
        }
    }

    /// Returns the color bit depth (0, 4, 8, or 24).
    pub fn bit_depth(&self) -> u8 {
        match self {
            Self::NoColor => 0,
            Self::Ansi16 => 4,
            Self::Ansi256 => 8,
            Self::TrueColor => 24,
        }
    }

    /// Detect terminal color capability by inspecting the live process environment.
    pub fn detect() -> Self {
        Self::detect_from(|key| std::env::var(key).ok())
    }

    /// Detect terminal color capability using a custom environment lookup closure.
    ///
    /// This allows deterministic testing and evaluation without mutating global state.
    pub fn detect_from<F>(get_var: F) -> Self
    where
        F: Fn(&str) -> Option<String>,
    {
        // 1. Explicit Fusion Override
        if let Some(val) = get_var("FUSION_COLOR_SUPPORT")
            .or_else(|| get_var("FUSION_COLOR_MODE"))
            .or_else(|| get_var("FUSION_COLORS"))
        {
            let lower = val.trim().to_ascii_lowercase();
            match lower.as_str() {
                "none" | "0" | "false" | "off" | "no" | "nocolor" | "no_color" | "monochrome" => {
                    return Self::NoColor;
                }
                "16" | "basic" | "ansi16" | "4bit" | "4-bit" | "ansi" => {
                    return Self::Ansi16;
                }
                "256" | "ansi256" | "8bit" | "8-bit" | "indexed" => {
                    return Self::Ansi256;
                }
                "truecolor" | "24bit" | "24-bit" | "rgb" | "direct" | "full" => {
                    return Self::TrueColor;
                }
                _ => {}
            }
        }

        // 2. FORCE_COLOR variable check
        // FORCE_COLOR="0" disables color
        // FORCE_COLOR="1" enables at least 16 colors (or higher if detected)
        // FORCE_COLOR="2" enables 256 colors
        // FORCE_COLOR="3" enables truecolor
        if let Some(force_color) = get_var("FORCE_COLOR") {
            let trimmed = force_color.trim().to_ascii_lowercase();
            if trimmed == "0" || trimmed == "false" || trimmed == "none" || trimmed == "off" {
                return Self::NoColor;
            } else if trimmed == "3" || trimmed == "truecolor" || trimmed == "24bit" {
                return Self::TrueColor;
            } else if trimmed == "2" || trimmed == "256" {
                return Self::Ansi256;
            } else if trimmed == "1" || trimmed == "true" || trimmed == "yes" || !trimmed.is_empty()
            {
                // If force_color is "1", at least ANSI 16 is guaranteed, but let's check if COLORTERM/TERM allows higher
                let detected = Self::detect_capabilities_without_force(&get_var);
                return if detected > Self::Ansi16 {
                    detected
                } else {
                    Self::Ansi16
                };
            }
        }

        // 3. NO_COLOR standard (https://no-color.org)
        // "Command-line software which accepts ANSI color settings should default to the normal,
        // uncolored display... when NO_COLOR is present in the environment (regardless of its value, as long as it is not empty)."
        if let Some(no_color) = get_var("NO_COLOR") {
            if !no_color.is_empty() {
                return Self::NoColor;
            }
        }

        // 4. CLICOLOR_FORCE and CLICOLOR standard
        // If CLICOLOR_FORCE != 0, color is forced.
        // If CLICOLOR == 0 and not forced, colors are disabled.
        let cli_forced = get_var("CLICOLOR_FORCE")
            .map(|v| v.trim() != "0" && !v.trim().is_empty())
            .unwrap_or(false);

        if !cli_forced {
            if let Some(clicolor) = get_var("CLICOLOR") {
                if clicolor.trim() == "0" {
                    return Self::NoColor;
                }
            }
        }

        // 5. Check TERM=dumb (unless CLICOLOR_FORCE is set)
        if !cli_forced {
            if let Some(term) = get_var("TERM") {
                if term.trim().eq_ignore_ascii_case("dumb") {
                    return Self::NoColor;
                }
            }
        }

        // 6. Detect through COLORTERM, TERM, TERM_PROGRAM, CI, Windows Terminal, etc.
        let detected = Self::detect_capabilities_without_force(&get_var);

        if cli_forced && detected == Self::NoColor {
            Self::Ansi16
        } else {
            detected
        }
    }

    /// Detect terminal color capability from a HashMap of environment variables.
    pub fn detect_from_env_map(map: &HashMap<String, String>) -> Self {
        Self::detect_from(|key| map.get(key).cloned())
    }

    /// Internal helper to evaluate COLORTERM, TERM, TERM_PROGRAM, CI, etc.
    fn detect_capabilities_without_force<F>(get_var: &F) -> Self
    where
        F: Fn(&str) -> Option<String>,
    {
        // A. Check COLORTERM
        // COLORTERM is standard for 24-bit / truecolor terminals (GNOME Terminal, iTerm2, Alacritty, Kitty, WezTerm, etc.)
        if let Some(colorterm) = get_var("COLORTERM") {
            let lower = colorterm.trim().to_ascii_lowercase();
            if lower == "truecolor" || lower == "24bit" || lower == "direct" {
                return Self::TrueColor;
            }
            if lower == "256color" || lower == "256" || lower == "yes" || lower == "true" {
                return Self::Ansi256;
            }
        }

        // B. Check TERM_PROGRAM
        if let Some(term_prog) = get_var("TERM_PROGRAM") {
            let lower = term_prog.trim().to_ascii_lowercase();
            match lower.as_str() {
                "iterm.app" | "iterm" | "vscode" | "hyper" | "wezterm" | "ghostty"
                | "warpterminal" | "warp" | "alacritty" | "kitty" | "rio" | "tabby" | "contour" => {
                    return Self::TrueColor;
                }
                "apple_terminal" => {
                    // Apple Terminal supports 256 colors
                    return Self::Ansi256;
                }
                _ => {}
            }
        }

        // C. Check Terminal specific environment markers
        if get_var("KITTY_WINDOW_ID").is_some()
            || get_var("ALACRITTY_LOG").is_some()
            || get_var("ALACRITTY_WINDOW_ID").is_some()
            || get_var("ALACRITTY_SOCKET").is_some()
            || get_var("GHOSTTY_RESOURCES_DIR").is_some()
            || get_var("WEZTERM_EXECUTABLE").is_some()
            || get_var("WEZTERM_PANE").is_some()
        {
            return Self::TrueColor;
        }

        // D. Windows Terminal / ConEmu / ANSICON
        if get_var("WT_SESSION").is_some() {
            // Windows Terminal has full TrueColor support
            return Self::TrueColor;
        }
        if let Some(conemu) = get_var("ConEmuANSI") {
            if conemu.eq_ignore_ascii_case("ON") {
                return Self::TrueColor;
            }
        }
        if get_var("ANSICON").is_some() {
            return Self::Ansi256;
        }

        // E. Termux Android Environment
        if get_var("TERMUX_VERSION").is_some() {
            return Self::TrueColor;
        }

        // F. CI Environments
        if get_var("GITHUB_ACTIONS").is_some() {
            // GitHub Actions runner terminal supports TrueColor
            return Self::TrueColor;
        }
        if get_var("GITLAB_CI").is_some() {
            return Self::Ansi256;
        }
        if get_var("TRAVIS").is_some()
            || get_var("CIRCLECI").is_some()
            || get_var("APPVEYOR").is_some()
            || get_var("BUILDKITE").is_some()
            || get_var("DRONE").is_some()
            || get_var("TEAMCITY_VERSION").is_some()
            || get_var("CI").is_some()
        {
            return Self::Ansi256;
        }

        // G. Check TERM environment variable
        if let Some(term) = get_var("TERM") {
            let lower = term.trim().to_ascii_lowercase();

            if lower == "dumb" {
                return Self::NoColor;
            }

            // Direct truecolor terms
            if lower.contains("truecolor") || lower.contains("24bit") || lower.contains("direct") {
                return Self::TrueColor;
            }

            // Modern terminals that are known to support TrueColor even without COLORTERM
            if lower.contains("alacritty")
                || lower.contains("kitty")
                || lower.contains("foot")
                || lower.contains("wezterm")
                || lower.contains("ghostty")
            {
                return Self::TrueColor;
            }

            // 256-color terms
            if lower.contains("256color")
                || lower.contains("256")
                || lower.contains("-256")
                || lower.ends_with("-256color")
            {
                return Self::Ansi256;
            }

            // Standard ANSI / xterm / screen / tmux / rxvt / vt100 / linux console
            if lower.starts_with("xterm")
                || lower.starts_with("screen")
                || lower.starts_with("tmux")
                || lower.starts_with("rxvt")
                || lower.starts_with("vt100")
                || lower.starts_with("vt220")
                || lower.starts_with("linux")
                || lower.starts_with("cygwin")
                || lower.starts_with("putty")
                || lower.contains("ansi")
                || lower.contains("color")
            {
                // Most modern xterm/tmux/screen setups support at least 256 colors or ANSI 16
                if lower.starts_with("xterm") || lower.starts_with("tmux") {
                    return Self::Ansi256;
                }
                return Self::Ansi16;
            }
        }

        // Default fallback for modern interactive environments: ANSI 16
        Self::Ansi16
    }
}

// ---------------------------------------------------------------------------
// Color Quantization & Conversion Utilities
// ---------------------------------------------------------------------------

/// Standard 16 ANSI colors RGB reference table.
pub const ANSI_16_RGB: [(u8, u8, u8); 16] = [
    (0, 0, 0),       // 0: Black
    (128, 0, 0),     // 1: Red
    (0, 128, 0),     // 2: Green
    (128, 128, 0),   // 3: Yellow
    (0, 0, 128),     // 4: Blue
    (128, 0, 128),   // 5: Magenta
    (0, 128, 128),   // 6: Cyan
    (192, 192, 192), // 7: Light Gray
    (128, 128, 128), // 8: Dark Gray
    (255, 0, 0),     // 9: Bright Red
    (0, 255, 0),     // 10: Bright Green
    (255, 255, 0),   // 11: Bright Yellow
    (0, 0, 255),     // 12: Bright Blue
    (255, 0, 255),   // 13: Bright Magenta
    (0, 255, 255),   // 14: Bright Cyan
    (255, 255, 255), // 15: Bright White
];

/// Step coordinates for xterm 6x6x6 color cube (indices 0..=5).
const CUBE_STEPS: [u8; 6] = [0, 95, 135, 175, 215, 255];

/// Convert an RGB color to the closest 16-color ANSI code (0..15).
pub fn rgb_to_ansi16(r: u8, g: u8, b: u8) -> u8 {
    let mut best_idx = 0;
    let mut min_dist = u32::MAX;

    for (idx, &(ar, ag, ab)) in ANSI_16_RGB.iter().enumerate() {
        let dr = (r as i32) - (ar as i32);
        let dg = (g as i32) - (ag as i32);
        let db = (b as i32) - (ab as i32);
        // Perceptual distance formula approximation
        let dist = (2 * dr * dr + 4 * dg * dg + 3 * db * db) as u32;
        if dist < min_dist {
            min_dist = dist;
            best_idx = idx as u8;
        }
    }

    best_idx
}

/// Convert an RGB color to the closest 256-color palette index (0..255).
pub fn rgb_to_ansi256(r: u8, g: u8, b: u8) -> u8 {
    let mut best_idx = 0;
    let mut min_dist = u32::MAX;

    // 1. Check standard 16 ANSI colors (indices 0..15)
    for (idx, &(ar, ag, ab)) in ANSI_16_RGB.iter().enumerate() {
        let dr = (r as i32) - (ar as i32);
        let dg = (g as i32) - (ag as i32);
        let db = (b as i32) - (ab as i32);
        let dist = (dr * dr + dg * dg + db * db) as u32;
        if dist < min_dist {
            min_dist = dist;
            best_idx = idx as u8;
        }
    }

    // 2. Check 6x6x6 Color Cube (indices 16..231)
    for ri in 0..6 {
        for gi in 0..6 {
            for bi in 0..6 {
                let cr = CUBE_STEPS[ri];
                let cg = CUBE_STEPS[gi];
                let cb = CUBE_STEPS[bi];

                let dr = (r as i32) - (cr as i32);
                let dg = (g as i32) - (cg as i32);
                let db = (b as i32) - (cb as i32);
                let dist = (dr * dr + dg * dg + db * db) as u32;

                if dist < min_dist {
                    min_dist = dist;
                    best_idx = 16 + (36 * ri + 6 * gi + bi) as u8;
                }
            }
        }
    }

    // 3. Check Grayscale Ramp (indices 232..255: 24 steps from 8 to 238, step 10)
    for i in 0..24 {
        let gray = 8 + (i as u8) * 10;
        let dr = (r as i32) - (gray as i32);
        let dg = (g as i32) - (gray as i32);
        let db = (b as i32) - (gray as i32);
        let dist = (dr * dr + dg * dg + db * db) as u32;

        if dist < min_dist {
            min_dist = dist;
            best_idx = 232 + i as u8;
        }
    }

    best_idx
}

/// Convert a 256-color palette index to its RGB coordinates.
pub fn ansi256_to_rgb(idx: u8) -> (u8, u8, u8) {
    match idx {
        0..=15 => ANSI_16_RGB[idx as usize],
        16..=231 => {
            let offset = idx - 16;
            let ri = (offset / 36) as usize;
            let gi = ((offset % 36) / 6) as usize;
            let bi = (offset % 6) as usize;
            (CUBE_STEPS[ri], CUBE_STEPS[gi], CUBE_STEPS[bi])
        }
        232..=255 => {
            let step = idx - 232;
            let gray = 8 + step * 10;
            (gray, gray, gray)
        }
    }
}

/// Convert a 16-color ANSI index (0..15) to its corresponding Ratatui `Color`.
pub fn ansi16_to_ratatui(idx: u8) -> Color {
    match idx {
        0 => Color::Black,
        1 => Color::Red,
        2 => Color::Green,
        3 => Color::Yellow,
        4 => Color::Blue,
        5 => Color::Magenta,
        6 => Color::Cyan,
        7 => Color::Gray,
        8 => Color::DarkGray,
        9 => Color::LightRed,
        10 => Color::LightGreen,
        11 => Color::LightYellow,
        12 => Color::LightBlue,
        13 => Color::LightMagenta,
        14 => Color::LightCyan,
        _ => Color::White,
    }
}

/// Downsamples a Ratatui `Color` to match the target terminal's `ColorCapability`.
///
/// - `ColorCapability::NoColor`: Converts color to `Color::Reset`.
/// - `ColorCapability::Ansi16`: Converts 24-bit RGB or 256-color to the closest standard ANSI color.
/// - `ColorCapability::Ansi256`: Converts 24-bit RGB to the closest 256-color indexed palette entry.
/// - `ColorCapability::TrueColor`: Preserves 24-bit RGB, indexed, and standard ANSI colors unchanged.
pub fn downsample_color(color: Color, capability: ColorCapability) -> Color {
    match capability {
        ColorCapability::NoColor => Color::Reset,
        ColorCapability::TrueColor => color,
        ColorCapability::Ansi256 => match color {
            Color::Rgb(r, g, b) => Color::Indexed(rgb_to_ansi256(r, g, b)),
            other => other,
        },
        ColorCapability::Ansi16 => match color {
            Color::Rgb(r, g, b) => ansi16_to_ratatui(rgb_to_ansi16(r, g, b)),
            Color::Indexed(idx) => {
                if idx < 16 {
                    ansi16_to_ratatui(idx)
                } else {
                    let (r, g, b) = ansi256_to_rgb(idx);
                    ansi16_to_ratatui(rgb_to_ansi16(r, g, b))
                }
            }
            other => other,
        },
    }
}

// ---------------------------------------------------------------------------
// ANSI Escape Code Formatting Helpers
// ---------------------------------------------------------------------------

/// Generate ANSI escape code for foreground RGB color adapted to the terminal's capability.
pub fn format_fg_escape(r: u8, g: u8, b: u8, capability: ColorCapability) -> String {
    match capability {
        ColorCapability::NoColor => String::new(),
        ColorCapability::TrueColor => format!("\x1b[38;2;{};{};{}m", r, g, b),
        ColorCapability::Ansi256 => {
            let idx = rgb_to_ansi256(r, g, b);
            format!("\x1b[38;5;{}m", idx)
        }
        ColorCapability::Ansi16 => {
            let idx = rgb_to_ansi16(r, g, b);
            if idx < 8 {
                format!("\x1b[{}m", 30 + idx)
            } else {
                format!("\x1b[{}m", 90 + (idx - 8))
            }
        }
    }
}

/// Generate ANSI escape code for background RGB color adapted to the terminal's capability.
pub fn format_bg_escape(r: u8, g: u8, b: u8, capability: ColorCapability) -> String {
    match capability {
        ColorCapability::NoColor => String::new(),
        ColorCapability::TrueColor => format!("\x1b[48;2;{};{};{}m", r, g, b),
        ColorCapability::Ansi256 => {
            let idx = rgb_to_ansi256(r, g, b);
            format!("\x1b[48;5;{}m", idx)
        }
        ColorCapability::Ansi16 => {
            let idx = rgb_to_ansi16(r, g, b);
            if idx < 8 {
                format!("\x1b[{}m", 40 + idx)
            } else {
                format!("\x1b[{}m", 100 + (idx - 8))
            }
        }
    }
}

/// ANSI reset code string (`\x1b[0m`).
pub const ANSI_RESET: &str = "\x1b[0m";

/// Strip ANSI escape sequences from a string.
pub fn strip_ansi(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut in_escape = false;

    for c in s.chars() {
        if in_escape {
            if c.is_ascii_alphabetic() || c == 'm' {
                in_escape = false;
            }
        } else if c == '\x1b' {
            in_escape = true;
        } else {
            result.push(c);
        }
    }

    result
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_color_env() {
        // NO_COLOR non-empty => NoColor
        let cap = ColorCapability::detect_from(|k| {
            if k == "NO_COLOR" {
                Some("1".to_string())
            } else if k == "COLORTERM" {
                Some("truecolor".to_string())
            } else {
                None
            }
        });
        assert_eq!(cap, ColorCapability::NoColor);
        assert!(!cap.has_color());
        assert!(cap.is_no_color());

        // NO_COLOR empty string is ignored
        let cap = ColorCapability::detect_from(|k| {
            if k == "NO_COLOR" {
                Some("".to_string())
            } else if k == "COLORTERM" {
                Some("truecolor".to_string())
            } else {
                None
            }
        });
        assert_eq!(cap, ColorCapability::TrueColor);
    }

    #[test]
    fn test_colorterm_truecolor() {
        for val in &["truecolor", "24bit", "direct", "TRUECOLOR", "24Bit"] {
            let cap = ColorCapability::detect_from(|k| {
                if k == "COLORTERM" {
                    Some(val.to_string())
                } else {
                    None
                }
            });
            assert_eq!(cap, ColorCapability::TrueColor);
            assert!(cap.is_truecolor());
            assert!(cap.has_color());
            assert_eq!(cap.bit_depth(), 24);
            assert_eq!(cap.max_colors(), 16_777_216);
        }
    }

    #[test]
    fn test_colorterm_256color() {
        let cap = ColorCapability::detect_from(|k| {
            if k == "COLORTERM" {
                Some("256color".to_string())
            } else {
                None
            }
        });
        assert_eq!(cap, ColorCapability::Ansi256);
        assert!(cap.is_256_color());
        assert_eq!(cap.bit_depth(), 8);
        assert_eq!(cap.max_colors(), 256);
    }

    #[test]
    fn test_term_detection() {
        // dumb => NoColor
        let cap = ColorCapability::detect_from(|k| {
            if k == "TERM" {
                Some("dumb".to_string())
            } else {
                None
            }
        });
        assert_eq!(cap, ColorCapability::NoColor);

        // xterm-256color => Ansi256
        let cap = ColorCapability::detect_from(|k| {
            if k == "TERM" {
                Some("xterm-256color".to_string())
            } else {
                None
            }
        });
        assert_eq!(cap, ColorCapability::Ansi256);

        // screen-256color => Ansi256
        let cap = ColorCapability::detect_from(|k| {
            if k == "TERM" {
                Some("screen-256color".to_string())
            } else {
                None
            }
        });
        assert_eq!(cap, ColorCapability::Ansi256);

        // alacritty => TrueColor
        let cap = ColorCapability::detect_from(|k| {
            if k == "TERM" {
                Some("alacritty".to_string())
            } else {
                None
            }
        });
        assert_eq!(cap, ColorCapability::TrueColor);

        // vt100 => Ansi16
        let cap = ColorCapability::detect_from(|k| {
            if k == "TERM" {
                Some("vt100".to_string())
            } else {
                None
            }
        });
        assert_eq!(cap, ColorCapability::Ansi16);
    }

    #[test]
    fn test_force_color() {
        // FORCE_COLOR=0 disables
        let cap = ColorCapability::detect_from(|k| {
            if k == "FORCE_COLOR" {
                Some("0".to_string())
            } else if k == "COLORTERM" {
                Some("truecolor".to_string())
            } else {
                None
            }
        });
        assert_eq!(cap, ColorCapability::NoColor);

        // FORCE_COLOR=1 with no other hints gives at least Ansi16
        let cap = ColorCapability::detect_from(|k| {
            if k == "FORCE_COLOR" {
                Some("1".to_string())
            } else {
                None
            }
        });
        assert_eq!(cap, ColorCapability::Ansi16);

        // FORCE_COLOR=2 gives 256
        let cap = ColorCapability::detect_from(|k| {
            if k == "FORCE_COLOR" {
                Some("2".to_string())
            } else {
                None
            }
        });
        assert_eq!(cap, ColorCapability::Ansi256);

        // FORCE_COLOR=3 gives TrueColor
        let cap = ColorCapability::detect_from(|k| {
            if k == "FORCE_COLOR" {
                Some("3".to_string())
            } else {
                None
            }
        });
        assert_eq!(cap, ColorCapability::TrueColor);
    }

    #[test]
    fn test_term_program_detection() {
        // iTerm.app => TrueColor
        let cap = ColorCapability::detect_from(|k| {
            if k == "TERM_PROGRAM" {
                Some("iTerm.app".to_string())
            } else {
                None
            }
        });
        assert_eq!(cap, ColorCapability::TrueColor);

        // Apple_Terminal => Ansi256
        let cap = ColorCapability::detect_from(|k| {
            if k == "TERM_PROGRAM" {
                Some("Apple_Terminal".to_string())
            } else {
                None
            }
        });
        assert_eq!(cap, ColorCapability::Ansi256);
    }

    #[test]
    fn test_ci_detection() {
        // GitHub Actions => TrueColor
        let cap = ColorCapability::detect_from(|k| {
            if k == "GITHUB_ACTIONS" {
                Some("true".to_string())
            } else {
                None
            }
        });
        assert_eq!(cap, ColorCapability::TrueColor);

        // GitLab CI => Ansi256
        let cap = ColorCapability::detect_from(|k| {
            if k == "GITLAB_CI" {
                Some("true".to_string())
            } else {
                None
            }
        });
        assert_eq!(cap, ColorCapability::Ansi256);
    }

    #[test]
    fn test_fusion_color_override() {
        let cap = ColorCapability::detect_from(|k| {
            if k == "FUSION_COLOR_SUPPORT" {
                Some("256".to_string())
            } else {
                None
            }
        });
        assert_eq!(cap, ColorCapability::Ansi256);

        let cap = ColorCapability::detect_from(|k| {
            if k == "FUSION_COLOR_SUPPORT" {
                Some("none".to_string())
            } else {
                None
            }
        });
        assert_eq!(cap, ColorCapability::NoColor);
    }

    #[test]
    fn test_rgb_to_ansi16() {
        assert_eq!(rgb_to_ansi16(0, 0, 0), 0); // Black
        assert_eq!(rgb_to_ansi16(255, 0, 0), 9); // Bright Red (or 1)
        assert_eq!(rgb_to_ansi16(0, 255, 0), 10); // Bright Green (or 2)
        assert_eq!(rgb_to_ansi16(0, 0, 255), 12); // Bright Blue (or 4)
        assert_eq!(rgb_to_ansi16(255, 255, 255), 15); // Bright White
    }

    #[test]
    fn test_rgb_to_ansi256_and_roundtrip() {
        assert_eq!(rgb_to_ansi256(0, 0, 0), 0);
        assert_eq!(rgb_to_ansi256(255, 255, 255), 15); // or 231

        // Test primary pure colors in cube
        let red_idx = rgb_to_ansi256(255, 0, 0);
        let (r, g, b) = ansi256_to_rgb(red_idx);
        assert_eq!(r, 255);
        assert_eq!(g, 0);
        assert_eq!(b, 0);

        // Test grayscale
        let gray_idx = rgb_to_ansi256(128, 128, 128);
        assert!(gray_idx == 8 || (232..=255).contains(&gray_idx));
    }

    #[test]
    fn test_downsample_color() {
        let rgb_color = Color::Rgb(125, 207, 255);

        // TrueColor keeps RGB
        assert_eq!(
            downsample_color(rgb_color, ColorCapability::TrueColor),
            rgb_color
        );

        // Ansi256 converts to Indexed
        match downsample_color(rgb_color, ColorCapability::Ansi256) {
            Color::Indexed(_) => {}
            other => panic!("Expected Color::Indexed, got {:?}", other),
        }

        // Ansi16 converts to standard ANSI Color variant
        match downsample_color(rgb_color, ColorCapability::Ansi16) {
            Color::Cyan | Color::LightCyan | Color::Blue | Color::LightBlue => {}
            other => panic!("Expected ANSI 16 color, got {:?}", other),
        }

        // NoColor converts to Reset
        assert_eq!(
            downsample_color(rgb_color, ColorCapability::NoColor),
            Color::Reset
        );
    }

    #[test]
    fn test_format_escape_codes() {
        let truecolor_fg = format_fg_escape(255, 100, 50, ColorCapability::TrueColor);
        assert_eq!(truecolor_fg, "\x1b[38;2;255;100;50m");

        let nocolor_fg = format_fg_escape(255, 100, 50, ColorCapability::NoColor);
        assert_eq!(nocolor_fg, "");

        let ansi256_fg = format_fg_escape(255, 0, 0, ColorCapability::Ansi256);
        assert!(ansi256_fg.starts_with("\x1b[38;5;"));

        let ansi16_fg = format_fg_escape(255, 0, 0, ColorCapability::Ansi16);
        assert!(ansi16_fg.starts_with("\x1b[3") || ansi16_fg.starts_with("\x1b[9"));
    }

    #[test]
    fn test_strip_ansi() {
        let styled = "\x1b[38;2;255;0;0mHello\x1b[0m World";
        assert_eq!(strip_ansi(styled), "Hello World");
    }
}
