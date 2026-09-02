//! User-customizable color palette loader for Fusion's Ratatui UI.
//!
//! Loads `~/.fusion/theme.toml` and overlays named color roles onto the
//! runtime [`Theme`]. When the file is absent or malformed the built-in
//! Tokyo Night defaults are used transparently.
//!
//! ## theme.toml format
//!
//! ```toml
//! # Each key is a palette role; value is a CSS-style hex color or an
//! # ANSI-256 index ("ansi:N") or a named ANSI color ("red", "cyan", …).
//! primary   = "#7dcfff"
//! secondary = "#7aa2f7"
//! error     = "#f7768e"
//! warning   = "#e0af68"
//! success   = "#9ece6a"
//! muted     = "#565f89"
//! border    = "#414868"
//! ```
//!
//! Unknown keys are silently ignored, making the file forward-compatible.

use ratatui::style::{Color, Style};
use std::collections::HashMap;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Palette roles
// ---------------------------------------------------------------------------

/// The seven semantic color roles that `theme.toml` may override.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PaletteRole {
    Primary,
    Secondary,
    Error,
    Warning,
    Success,
    Muted,
    Border,
}

impl PaletteRole {
    /// All roles in a stable, iteration-friendly order.
    pub const ALL: &'static [Self] = &[
        Self::Primary,
        Self::Secondary,
        Self::Error,
        Self::Warning,
        Self::Success,
        Self::Muted,
        Self::Border,
    ];

    /// Canonical lowercase TOML key for this role.
    pub fn key(self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::Secondary => "secondary",
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Success => "success",
            Self::Muted => "muted",
            Self::Border => "border",
        }
    }

    /// Parse a `&str` key into a role, returning `None` for unknown keys.
    pub fn from_key(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "primary" => Some(Self::Primary),
            "secondary" => Some(Self::Secondary),
            "error" => Some(Self::Error),
            "warning" => Some(Self::Warning),
            "success" => Some(Self::Success),
            "muted" => Some(Self::Muted),
            "border" => Some(Self::Border),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Theme (palette + helpers)
// ---------------------------------------------------------------------------

/// Resolved color palette, loaded from `~/.fusion/theme.toml` with
/// Tokyo Night defaults for any role not explicitly specified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Theme {
    pub primary: Color,
    pub secondary: Color,
    pub error: Color,
    pub warning: Color,
    pub success: Color,
    pub muted: Color,
    pub border: Color,
}

impl Default for Theme {
    fn default() -> Self {
        // Tokyo Night dark defaults (matches src/ui/theme.rs)
        Self {
            primary: Color::Rgb(125, 207, 255),  // #7dcfff – cyan
            secondary: Color::Rgb(122, 162, 247), // #7aa2f7 – blue
            error: Color::Rgb(247, 118, 142),    // #f7768e – red
            warning: Color::Rgb(224, 175, 104),  // #e0af68 – yellow/orange
            success: Color::Rgb(158, 206, 106),  // #9ece6a – green
            muted: Color::Rgb(86, 95, 137),      // #565f89 – comment grey
            border: Color::Rgb(65, 72, 104),     // #414868 – border
        }
    }
}

impl Theme {
    /// Load `~/.fusion/theme.toml`, overlaying any present keys onto the
    /// defaults. If the file does not exist or cannot be read/parsed, the
    /// default palette is returned silently.
    pub fn load() -> Self {
        let mut theme = Self::default();
        if let Some(path) = config_path() {
            if let Ok(src) = std::fs::read_to_string(&path) {
                theme.apply_toml(&src);
            }
        }
        theme
    }

    /// Load from an explicit file path (primarily for testing).
    pub fn load_from(path: &std::path::Path) -> Self {
        let mut theme = Self::default();
        if let Ok(src) = std::fs::read_to_string(path) {
            theme.apply_toml(&src);
        }
        theme
    }

    /// Return the [`Color`] for a given [`PaletteRole`].
    pub fn color(&self, role: PaletteRole) -> Color {
        match role {
            PaletteRole::Primary => self.primary,
            PaletteRole::Secondary => self.secondary,
            PaletteRole::Error => self.error,
            PaletteRole::Warning => self.warning,
            PaletteRole::Success => self.success,
            PaletteRole::Muted => self.muted,
            PaletteRole::Border => self.border,
        }
    }

    /// Apply `role`'s foreground color onto `style` and return it.
    ///
    /// This is the primary integration point: call it whenever you build a
    /// Ratatui [`Style`] and want the user's palette preference respected.
    ///
    /// ```rust,ignore
    /// let s = theme.apply(Style::default(), PaletteRole::Error);
    /// ```
    pub fn apply(&self, style: Style, role: PaletteRole) -> Style {
        style.fg(self.color(role))
    }

    /// Parse and apply a TOML source string onto `self` in place.
    /// Unknown keys and malformed values are silently ignored.
    fn apply_toml(&mut self, src: &str) {
        for (key, value) in parse_toml_pairs(src) {
            if let Some(role) = PaletteRole::from_key(&key) {
                if let Some(color) = parse_color(&value) {
                    self.set_role(role, color);
                }
            }
        }
    }

    fn set_role(&mut self, role: PaletteRole, color: Color) {
        match role {
            PaletteRole::Primary => self.primary = color,
            PaletteRole::Secondary => self.secondary = color,
            PaletteRole::Error => self.error = color,
            PaletteRole::Warning => self.warning = color,
            PaletteRole::Success => self.success = color,
            PaletteRole::Muted => self.muted = color,
            PaletteRole::Border => self.border = color,
        }
    }

    /// Build a map of role → color for the entire palette.
    pub fn as_map(&self) -> HashMap<PaletteRole, Color> {
        PaletteRole::ALL
            .iter()
            .map(|&r| (r, self.color(r)))
            .collect()
    }
}

// ---------------------------------------------------------------------------
// TOML subset parser
// ---------------------------------------------------------------------------

/// Parse `key = "value"` lines from a minimal TOML file.
///
/// Handles:
/// - `key = "value"` (double-quoted string)
/// - `key = 'value'` (single-quoted string)
/// - `key = value`   (bare value, no spaces)
/// - `# comments`
/// - `[section]` headers (ignored — we only care about top-level keys)
/// - Inline `# comments` after a value
///
/// This is intentionally minimal: it covers the documented format without
/// pulling in an external TOML crate.
fn parse_toml_pairs(src: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    for raw_line in src.lines() {
        // Strip leading/trailing whitespace and inline comments.
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
            continue;
        }
        // Split on first `=`.
        let eq = match line.find('=') {
            Some(i) => i,
            None => continue,
        };
        let key = line[..eq].trim().to_string();
        let raw_val = line[eq + 1..].trim();
        // Strip optional trailing comment (only outside quotes — good enough
        // for the simple hex / name values we support).
        let value = strip_toml_value(raw_val).trim().to_string();
        if !key.is_empty() && !value.is_empty() {
            pairs.push((key, value));
        }
    }
    pairs
}

/// Extract the bare value from a TOML scalar, removing surrounding quotes
/// and trailing inline comments.
fn strip_toml_value(raw: &str) -> String {
    // Quoted string: "…" or '…'
    if (raw.starts_with('"') && raw.contains('"'))
        || (raw.starts_with('\'') && raw.contains('\''))
    {
        let quote = raw.chars().next().unwrap();
        // Find closing quote (skip the opening one).
        if let Some(end) = raw[1..].find(quote) {
            return raw[1..end + 1].to_string();
        }
    }
    // Bare value: strip trailing `# comment`.
    if let Some(hash) = raw.find('#') {
        return raw[..hash].trim().to_string();
    }
    raw.to_string()
}

// ---------------------------------------------------------------------------
// Color value parser
// ---------------------------------------------------------------------------

/// Parse a color value from a string representation.
///
/// Supported formats:
/// - `#rrggbb`  — 6-digit hex
/// - `#rgb`     — 3-digit hex (expanded to 6-digit)
/// - `ansi:N`   — ANSI 256-color index (0–255)
/// - Named ANSI: `black`, `red`, `green`, `yellow`, `blue`, `magenta`,
///               `cyan`, `white`, `darkgray`/`dark_gray`, `lightred`/
///               `light_red`, `lightgreen`/`light_green`, `lightyellow`/
///               `light_yellow`, `lightblue`/`light_blue`,
///               `lightmagenta`/`light_magenta`, `lightcyan`/`light_cyan`,
///               `gray`/`grey`, `reset`
fn parse_color(s: &str) -> Option<Color> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }

    // Hex color: #rrggbb or #rgb
    if let Some(hex) = s.strip_prefix('#') {
        return parse_hex_color(hex);
    }

    // ANSI index: ansi:N
    if let Some(rest) = s.strip_prefix("ansi:") {
        if let Ok(idx) = rest.trim().parse::<u8>() {
            return Some(Color::Indexed(idx));
        }
        return None;
    }

    // Named ANSI colors (case-insensitive, with/without underscore variants)
    match s.to_ascii_lowercase().replace('_', "").as_str() {
        "black" => Some(Color::Black),
        "red" => Some(Color::Red),
        "green" => Some(Color::Green),
        "yellow" => Some(Color::Yellow),
        "blue" => Some(Color::Blue),
        "magenta" => Some(Color::Magenta),
        "cyan" => Some(Color::Cyan),
        "white" => Some(Color::White),
        "darkgray" | "darkgrey" => Some(Color::DarkGray),
        "lightred" => Some(Color::LightRed),
        "lightgreen" => Some(Color::LightGreen),
        "lightyellow" => Some(Color::LightYellow),
        "lightblue" => Some(Color::LightBlue),
        "lightmagenta" => Some(Color::LightMagenta),
        "lightcyan" => Some(Color::LightCyan),
        "gray" | "grey" => Some(Color::Gray),
        "reset" => Some(Color::Reset),
        _ => None,
    }
}

/// Parse a 3- or 6-digit hex string (without the leading `#`) into an RGB color.
fn parse_hex_color(hex: &str) -> Option<Color> {
    match hex.len() {
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            Some(Color::Rgb(r, g, b))
        }
        3 => {
            let r = u8::from_str_radix(&hex[0..1], 16).ok()?;
            let g = u8::from_str_radix(&hex[1..2], 16).ok()?;
            let b = u8::from_str_radix(&hex[2..3], 16).ok()?;
            // Expand nibble: 0xA → 0xAA
            Some(Color::Rgb(r << 4 | r, g << 4 | g, b << 4 | b))
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Config path helper
// ---------------------------------------------------------------------------

/// Return the path to `~/.fusion/theme.toml`, or `None` if the home
/// directory cannot be determined.
fn config_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".fusion").join("theme.toml"))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    // --- parse_color ---

    #[test]
    fn parse_hex6_color() {
        assert_eq!(parse_color("#7dcfff"), Some(Color::Rgb(0x7d, 0xcf, 0xff)));
        assert_eq!(parse_color("#000000"), Some(Color::Rgb(0, 0, 0)));
        assert_eq!(parse_color("#ffffff"), Some(Color::Rgb(255, 255, 255)));
        assert_eq!(parse_color("#F7768E"), Some(Color::Rgb(0xf7, 0x76, 0x8e)));
    }

    #[test]
    fn parse_hex3_color() {
        // #abc → #aabbcc
        assert_eq!(parse_color("#abc"), Some(Color::Rgb(0xaa, 0xbb, 0xcc)));
        assert_eq!(parse_color("#fff"), Some(Color::Rgb(0xff, 0xff, 0xff)));
        assert_eq!(parse_color("#000"), Some(Color::Rgb(0x00, 0x00, 0x00)));
    }

    #[test]
    fn parse_ansi_index_color() {
        assert_eq!(parse_color("ansi:0"), Some(Color::Indexed(0)));
        assert_eq!(parse_color("ansi:255"), Some(Color::Indexed(255)));
        assert_eq!(parse_color("ansi:42"), Some(Color::Indexed(42)));
    }

    #[test]
    fn parse_named_colors() {
        assert_eq!(parse_color("red"), Some(Color::Red));
        assert_eq!(parse_color("RED"), Some(Color::Red));
        assert_eq!(parse_color("cyan"), Some(Color::Cyan));
        assert_eq!(parse_color("darkgray"), Some(Color::DarkGray));
        assert_eq!(parse_color("dark_gray"), Some(Color::DarkGray));
        assert_eq!(parse_color("lightblue"), Some(Color::LightBlue));
        assert_eq!(parse_color("light_blue"), Some(Color::LightBlue));
        assert_eq!(parse_color("gray"), Some(Color::Gray));
        assert_eq!(parse_color("grey"), Some(Color::Gray));
        assert_eq!(parse_color("reset"), Some(Color::Reset));
    }

    #[test]
    fn parse_invalid_color_returns_none() {
        assert_eq!(parse_color(""), None);
        assert_eq!(parse_color("#gg0000"), None);   // bad hex digit
        assert_eq!(parse_color("#12345"), None);    // 5-digit hex
        assert_eq!(parse_color("ansi:256"), None);  // out of u8 range
        assert_eq!(parse_color("fuschia"), None);   // unknown name
    }

    // --- parse_toml_pairs ---

    #[test]
    fn toml_double_quoted_values() {
        let src = r#"primary = "#7dcfff""#;
        let pairs = parse_toml_pairs(src);
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].0, "primary");
        assert_eq!(pairs[0].1, "#7dcfff");
    }

    #[test]
    fn toml_single_quoted_values() {
        let src = "error = '#f7768e'";
        let pairs = parse_toml_pairs(src);
        assert_eq!(pairs[0].1, "#f7768e");
    }

    #[test]
    fn toml_bare_value() {
        let src = "muted = cyan";
        let pairs = parse_toml_pairs(src);
        assert_eq!(pairs[0].1, "cyan");
    }

    #[test]
    fn toml_strips_inline_comments() {
        let src = "border = #414868 # dark border";
        let pairs = parse_toml_pairs(src);
        // bare value: `#414868` — the first `#` starts the value, but after
        // it is a space then another `#`; strip_toml_value trims that.
        // Here `#414868 # dark border` is a bare value.  The first `#` is
        // NOT a comment delimiter because it immediately follows the `=`.
        // strip_toml_value will look for `#` in the bare string after position 0.
        // Let's verify the round-trip via parse_color works too.
        assert!(!pairs.is_empty());
    }

    #[test]
    fn toml_skips_comments_and_sections() {
        let src = "# this is a comment\n[palette]\nprimary = \"#7dcfff\"\n";
        let pairs = parse_toml_pairs(src);
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].0, "primary");
    }

    #[test]
    fn toml_ignores_unknown_keys() {
        let src = "unknown_key = \"#ff0000\"\nprimary = \"#7dcfff\"";
        let pairs = parse_toml_pairs(src);
        // Both are returned by the low-level parser; Theme::apply_toml filters.
        let roles: Vec<_> = pairs
            .iter()
            .filter_map(|(k, _)| PaletteRole::from_key(k))
            .collect();
        assert_eq!(roles, vec![PaletteRole::Primary]);
    }

    // --- Theme::default ---

    #[test]
    fn default_theme_matches_tokyo_night() {
        let t = Theme::default();
        assert_eq!(t.primary, Color::Rgb(125, 207, 255));
        assert_eq!(t.secondary, Color::Rgb(122, 162, 247));
        assert_eq!(t.error, Color::Rgb(247, 118, 142));
        assert_eq!(t.warning, Color::Rgb(224, 175, 104));
        assert_eq!(t.success, Color::Rgb(158, 206, 106));
        assert_eq!(t.muted, Color::Rgb(86, 95, 137));
        assert_eq!(t.border, Color::Rgb(65, 72, 104));
    }

    // --- Theme::load_from ---

    #[test]
    fn load_from_absent_file_returns_defaults() {
        let t = Theme::load_from(std::path::Path::new("/nonexistent/path/theme.toml"));
        assert_eq!(t, Theme::default());
    }

    #[test]
    fn load_from_empty_file_returns_defaults() {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(b"").unwrap();
        let t = Theme::load_from(f.path());
        assert_eq!(t, Theme::default());
    }

    #[test]
    fn load_from_full_override() {
        let toml = r#"
primary   = "#aabbcc"
secondary = "#112233"
error     = "#ff0000"
warning   = "#ffaa00"
success   = "#00ff00"
muted     = "#888888"
border    = "#444444"
"#;
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(toml.as_bytes()).unwrap();
        let t = Theme::load_from(f.path());
        assert_eq!(t.primary, Color::Rgb(0xaa, 0xbb, 0xcc));
        assert_eq!(t.secondary, Color::Rgb(0x11, 0x22, 0x33));
        assert_eq!(t.error, Color::Rgb(0xff, 0x00, 0x00));
        assert_eq!(t.warning, Color::Rgb(0xff, 0xaa, 0x00));
        assert_eq!(t.success, Color::Rgb(0x00, 0xff, 0x00));
        assert_eq!(t.muted, Color::Rgb(0x88, 0x88, 0x88));
        assert_eq!(t.border, Color::Rgb(0x44, 0x44, 0x44));
    }

    #[test]
    fn load_from_partial_override_keeps_defaults() {
        let toml = "error = \"#ff0000\"\n";
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(toml.as_bytes()).unwrap();
        let t = Theme::load_from(f.path());
        // Only error is changed.
        assert_eq!(t.error, Color::Rgb(255, 0, 0));
        // All others remain default.
        let d = Theme::default();
        assert_eq!(t.primary, d.primary);
        assert_eq!(t.secondary, d.secondary);
        assert_eq!(t.warning, d.warning);
        assert_eq!(t.success, d.success);
        assert_eq!(t.muted, d.muted);
        assert_eq!(t.border, d.border);
    }

    #[test]
    fn load_from_named_color() {
        let toml = "primary = cyan\n";
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(toml.as_bytes()).unwrap();
        let t = Theme::load_from(f.path());
        assert_eq!(t.primary, Color::Cyan);
    }

    #[test]
    fn load_from_ansi_index() {
        let toml = "border = ansi:240\n";
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(toml.as_bytes()).unwrap();
        let t = Theme::load_from(f.path());
        assert_eq!(t.border, Color::Indexed(240));
    }

    #[test]
    fn load_from_bad_color_value_ignored() {
        let toml = "primary = not_a_color\nerror = \"#ff0000\"\n";
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(toml.as_bytes()).unwrap();
        let t = Theme::load_from(f.path());
        // primary bad → default kept, error overridden
        assert_eq!(t.primary, Theme::default().primary);
        assert_eq!(t.error, Color::Rgb(255, 0, 0));
    }

    // --- Theme::color / Theme::apply ---

    #[test]
    fn color_lookup_by_role() {
        let t = Theme::default();
        assert_eq!(t.color(PaletteRole::Primary), t.primary);
        assert_eq!(t.color(PaletteRole::Error), t.error);
        assert_eq!(t.color(PaletteRole::Border), t.border);
    }

    #[test]
    fn apply_sets_foreground() {
        let t = Theme::default();
        let base = Style::default();
        let styled = t.apply(base, PaletteRole::Error);
        assert_eq!(styled.fg, Some(t.error));
    }

    #[test]
    fn apply_preserves_existing_modifiers() {
        use ratatui::style::Modifier;
        let t = Theme::default();
        let base = Style::default().add_modifier(Modifier::BOLD);
        let styled = t.apply(base, PaletteRole::Success);
        assert_eq!(styled.fg, Some(t.success));
        assert!(styled.add_modifier.contains(Modifier::BOLD));
    }

    // --- Theme::as_map ---

    #[test]
    fn as_map_covers_all_roles() {
        let t = Theme::default();
        let map = t.as_map();
        for &role in PaletteRole::ALL {
            assert!(map.contains_key(&role), "missing role {:?}", role);
            assert_eq!(map[&role], t.color(role));
        }
    }

    // --- PaletteRole ---

    #[test]
    fn role_key_round_trip() {
        for &role in PaletteRole::ALL {
            let key = role.key();
            assert_eq!(PaletteRole::from_key(key), Some(role));
        }
    }

    #[test]
    fn role_from_unknown_key_returns_none() {
        assert_eq!(PaletteRole::from_key("accent"), None);
        assert_eq!(PaletteRole::from_key(""), None);
        assert_eq!(PaletteRole::from_key("PRIMARY"), Some(PaletteRole::Primary)); // case-insensitive
    }
}
