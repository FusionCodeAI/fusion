//! Print and PDF stylesheets and generators for exported session HTML transcripts.
//!
//! Provides a zero-dependency, cross-platform engine for generating clean, highly legible,
//! page-budget-conscious print and PDF stylesheets for AI session transcripts, technical docs,
//! and tool execution logs.
//!
//! Key capabilities:
//! - Complete `@media print` stylesheets and `@page` rules for all standard paper formats
//!   (A4, Letter, Legal, Tabloid, A3, A5, and custom millimeter/inch dimensions)
//! - Support for multiple print themes:
//!   - `CleanLight`: Default subtle slate borders and shaded code blocks optimized for laser/inkjet
//!   - `Monochrome`: 100% ink-saving pure black-and-white for low-cost monochrome printing
//!   - `HighContrast`: Deep blacks, bold headings, and high legibility for accessibility
//!   - `ColorPreserved`: Retains syntax highlighting colors and badge hues on crisp white paper
//!   - `Minimalist`: Borderless typography-focused layout with clean vertical rhythm
//! - Page break management: prevents awkward splits inside message cards, code blocks,
//!   tool calls, tables, blockquotes, and reasoning `<think>` blocks (`break-inside: avoid`)
//! - Orphan (`orphans: 3`) and widow (`widows: 3`) control for flowing paragraphs
//! - Paged media running headers and footers with CSS counters (`counter(page)`, `counter(pages)`)
//!   compatible with browser print dialogs, Chromium Headless, WeasyPrint, and PrinceXML
//! - Automatic `<details open>` expansion for collapsible tool invocations and think blocks
//! - Suppression of interactive UI noise (copy buttons, search filters, audio widgets, scrollbars)
//! - Optional print URL expansion (`a[href]:after { content: " (" attr(href) ")"; }`)
//! - Utility functions for embedding print styles or optimizing HTML documents for PDF conversion.

use std::fmt::Write as _;

// ============================================================================
// Core Constants
// ============================================================================

/// Default print CSS applied within `@media print` blocks.
pub const DEFAULT_PRINT_CSS: &str = r#"
/* Fusion Default Print Stylesheet */
@media print {
  *,
  *::before,
  *::after {
    box-sizing: border-box !important;
    text-shadow: none !important;
    box-shadow: none !important;
    -webkit-print-color-adjust: exact !important;
    print-color-adjust: exact !important;
  }

  html,
  body {
    background: #ffffff !important;
    color: #111827 !important;
    font-size: 10pt !important;
    line-height: 1.5 !important;
    padding: 0 !important;
    margin: 0 !important;
    width: 100% !important;
  }

  /* Hide interactive and non-printable UI components */
  .header-actions,
  .toolbar,
  .filter-group,
  .search-box,
  .copy-btn,
  .copy-code-btn,
  .theme-toggle,
  .audio-player,
  .export-dropdown,
  .floating-action-button,
  .no-print,
  [data-no-print="true"] {
    display: none !important;
    visibility: hidden !important;
  }

  /* Force details elements open so content is never hidden in print */
  details {
    display: block !important;
  }

  details > summary {
    list-style: none !important;
    cursor: default !important;
    font-weight: 600 !important;
    padding-bottom: 4pt !important;
  }

  details > summary::-webkit-details-marker {
    display: none !important;
  }

  /* Typography */
  h1, h2, h3, h4, h5, h6 {
    color: #0f172a !important;
    break-after: avoid !important;
    page-break-after: avoid !important;
    font-weight: 700 !important;
  }

  h1 { font-size: 16pt !important; margin-top: 0 !important; margin-bottom: 8pt !important; }
  h2 { font-size: 13pt !important; margin-top: 14pt !important; margin-bottom: 6pt !important; }
  h3 { font-size: 11pt !important; margin-top: 10pt !important; margin-bottom: 4pt !important; }

  p, li, blockquote {
    orphans: 3 !important;
    widows: 3 !important;
  }

  /* App container */
  .app-container {
    max-width: 100% !important;
    margin: 0 !important;
    padding: 0 !important;
    border: none !important;
    box-shadow: none !important;
  }

  /* Header card */
  .app-header {
    background: #ffffff !important;
    border: 1.5pt solid #cbd5e1 !important;
    border-radius: 4pt !important;
    padding: 10pt 12pt !important;
    margin-bottom: 12pt !important;
    page-break-inside: avoid !important;
    break-inside: avoid !important;
  }

  .session-title {
    font-size: 14pt !important;
    font-weight: 700 !important;
    color: #0f172a !important;
    margin-bottom: 4pt !important;
  }

  .metadata-bar,
  .stats-bar {
    font-size: 8.5pt !important;
    color: #475569 !important;
    display: flex !important;
    flex-wrap: wrap !important;
    gap: 8pt !important;
    border-top: 0.5pt solid #e2e8f0 !important;
    padding-top: 6pt !important;
    margin-top: 6pt !important;
  }

  /* Message Cards */
  .timeline {
    display: block !important;
  }

  .message-card {
    background: #ffffff !important;
    border: 1pt solid #e2e8f0 !important;
    border-radius: 4pt !important;
    margin-bottom: 10pt !important;
    padding: 8pt 10pt !important;
    page-break-inside: avoid !important;
    break-inside: avoid !important;
  }

  .message-card.role-user {
    border-left: 3.5pt solid #3b82f6 !important;
    background: #f8fafc !important;
  }

  .message-card.role-assistant {
    border-left: 3.5pt solid #10b981 !important;
    background: #ffffff !important;
  }

  .message-card.role-system {
    border-left: 3.5pt solid #64748b !important;
    background: #f1f5f9 !important;
  }

  .message-card.role-tool {
    border-left: 3.5pt solid #f59e0b !important;
    background: #fffbeb !important;
  }

  .message-header {
    display: flex !important;
    justify-content: space-between !important;
    align-items: center !important;
    border-bottom: 0.5pt solid #e2e8f0 !important;
    padding-bottom: 4pt !important;
    margin-bottom: 6pt !important;
    font-size: 8.5pt !important;
    color: #475569 !important;
  }

  .role-badge {
    font-weight: 700 !important;
    font-size: 8pt !important;
    text-transform: uppercase !important;
    padding: 1.5pt 4pt !important;
    border-radius: 2pt !important;
    border: 0.5pt solid #cbd5e1 !important;
  }

  /* Reasoning / DeepSeek <think> blocks */
  .thinking-block,
  .thought-content,
  blockquote.think-block {
    background: #f8fafc !important;
    border-left: 2.5pt solid #94a3b8 !important;
    color: #334155 !important;
    font-style: italic !important;
    padding: 6pt 8pt !important;
    margin: 6pt 0 !important;
    border-radius: 0 3pt 3pt 0 !important;
    font-size: 9pt !important;
    page-break-inside: avoid !important;
    break-inside: avoid !important;
  }

  /* Tool execution blocks */
  .tool-call,
  .tool-result {
    background: #f8fafc !important;
    border: 1pt solid #cbd5e1 !important;
    border-radius: 3pt !important;
    padding: 6pt 8pt !important;
    margin: 6pt 0 !important;
    font-size: 8.5pt !important;
    page-break-inside: avoid !important;
    break-inside: avoid !important;
  }

  .tool-name {
    font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace !important;
    font-weight: 700 !important;
    color: #0f172a !important;
  }

  /* Code & Syntax Highlighting */
  pre,
  code {
    font-family: ui-monospace, SFMono-Regular, "Cascadia Code", "Source Code Pro", Menlo, Monaco, Consolas, monospace !important;
    font-size: 8.5pt !important;
    tab-size: 2 !important;
  }

  pre {
    background: #f8fafc !important;
    color: #0f172a !important;
    border: 1pt solid #cbd5e1 !important;
    border-radius: 3pt !important;
    padding: 6pt 8pt !important;
    margin: 6pt 0 !important;
    white-space: pre-wrap !important;
    word-break: break-word !important;
    overflow-wrap: break-word !important;
    page-break-inside: avoid !important;
    break-inside: avoid !important;
  }

  code:not(pre code) {
    background: #f1f5f9 !important;
    color: #0f172a !important;
    padding: 1pt 3pt !important;
    border-radius: 2pt !important;
    border: 0.5pt solid #e2e8f0 !important;
    font-size: 8.5pt !important;
  }

  /* Tables */
  table {
    width: 100% !important;
    border-collapse: collapse !important;
    margin: 8pt 0 !important;
    font-size: 8.5pt !important;
    page-break-inside: avoid !important;
    break-inside: avoid !important;
  }

  thead {
    display: table-header-group !important;
  }

  tr {
    page-break-inside: avoid !important;
    break-inside: avoid !important;
  }

  th, td {
    border: 0.75pt solid #cbd5e1 !important;
    padding: 4pt 6pt !important;
    text-align: left !important;
  }

  th {
    background: #f1f5f9 !important;
    font-weight: 700 !important;
    color: #0f172a !important;
  }

  tbody tr:nth-child(even) {
    background: #f8fafc !important;
  }

  /* Blockquotes */
  blockquote {
    border-left: 3pt solid #cbd5e1 !important;
    margin: 6pt 0 !important;
    padding: 4pt 8pt !important;
    color: #334155 !important;
    background: #f8fafc !important;
    page-break-inside: avoid !important;
    break-inside: avoid !important;
  }

  /* Links */
  a {
    color: #0f172a !important;
    text-decoration: underline !important;
  }

  /* Footer */
  .app-footer {
    border-top: 1pt solid #cbd5e1 !important;
    padding-top: 8pt !important;
    margin-top: 14pt !important;
    font-size: 8pt !important;
    color: #64748b !important;
    text-align: center !important;
    page-break-inside: avoid !important;
    break-inside: avoid !important;
  }
}
"#;

/// Default `@page` CSS rule setting A4 margins and running page numbers.
pub const DEFAULT_PAGE_CSS: &str = r#"
@page {
  size: A4 portrait;
  margin: 15mm 12mm 15mm 12mm;
  @bottom-right {
    content: "Page " counter(page) " of " counter(pages);
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-size: 8pt;
    color: #64748b;
  }
  @bottom-left {
    content: "Fusion AI Transcript";
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-size: 8pt;
    color: #94a3b8;
  }
}
"#;

// ============================================================================
// Enums & Configurations
// ============================================================================

/// Standard paper sizes supported for print and PDF generation.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum PageSize {
    /// International standard A4 (210mm x 297mm). Default for most regions.
    #[default]
    A4,
    /// North American standard US Letter (8.5in x 11in).
    Letter,
    /// North American standard US Legal (8.5in x 14in).
    Legal,
    /// North American Tabloid (11in x 17in).
    Tabloid,
    /// International standard A3 (297mm x 420mm).
    A3,
    /// International standard A5 (148mm x 210mm).
    A5,
    /// Executive (7.25in x 10.5in).
    Executive,
    /// Custom paper size with explicit dimensions.
    Custom { width: String, height: String },
}

impl PageSize {
    /// Returns the CSS `size` property value including orientation.
    pub fn to_css_value(&self, orientation: PageOrientation) -> String {
        let orient_str = orientation.as_css_str();
        match self {
            PageSize::A4 => format!("A4 {orient_str}"),
            PageSize::Letter => format!("letter {orient_str}"),
            PageSize::Legal => format!("legal {orient_str}"),
            PageSize::Tabloid => format!("tabloid {orient_str}"),
            PageSize::A3 => format!("A3 {orient_str}"),
            PageSize::A5 => format!("A5 {orient_str}"),
            PageSize::Executive => format!("executive {orient_str}"),
            PageSize::Custom { width, height } => {
                if orientation == PageOrientation::Landscape {
                    format!("{height} {width}")
                } else {
                    format!("{width} {height}")
                }
            }
        }
    }

    /// Approximate dimensions in millimeters `(width, height)` in portrait orientation.
    pub fn dimensions_mm(&self) -> Option<(f32, f32)> {
        match self {
            PageSize::A4 => Some((210.0, 297.0)),
            PageSize::Letter => Some((215.9, 279.4)),
            PageSize::Legal => Some((215.9, 355.6)),
            PageSize::Tabloid => Some((279.4, 431.8)),
            PageSize::A3 => Some((297.0, 420.0)),
            PageSize::A5 => Some((148.0, 210.0)),
            PageSize::Executive => Some((184.15, 266.7)),
            PageSize::Custom { .. } => None,
        }
    }

    /// User-friendly display name.
    pub fn display_name(&self) -> &'static str {
        match self {
            PageSize::A4 => "A4 (210 × 297 mm)",
            PageSize::Letter => "US Letter (8.5 × 11 in)",
            PageSize::Legal => "US Legal (8.5 × 14 in)",
            PageSize::Tabloid => "Tabloid (11 × 17 in)",
            PageSize::A3 => "A3 (297 × 420 mm)",
            PageSize::A5 => "A5 (148 × 210 mm)",
            PageSize::Executive => "Executive (7.25 × 10.5 in)",
            PageSize::Custom { .. } => "Custom Dimensions",
        }
    }
}

/// Paper orientation for printing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PageOrientation {
    /// Tall format (default).
    #[default]
    Portrait,
    /// Wide format (ideal for wide tables, logs, side-by-side diffs).
    Landscape,
}

impl PageOrientation {
    /// Returns the CSS keyword for the orientation.
    pub fn as_css_str(&self) -> &'static str {
        match self {
            PageOrientation::Portrait => "portrait",
            PageOrientation::Landscape => "landscape",
        }
    }
}

/// Page margins for print and PDF generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageMargin {
    /// Standard margins: 15mm top/bottom, 12mm left/right.
    Standard,
    /// Compact margins: 8mm all around (maximizes printable area).
    Compact,
    /// Wide margins: 25mm all around (room for annotations/binding).
    Wide,
    /// Zero margins (content touches page edges; useful for custom bleed layouts).
    None,
    /// Custom margin string per side.
    Custom {
        top: String,
        right: String,
        bottom: String,
        left: String,
    },
}

impl Default for PageMargin {
    fn default() -> Self {
        PageMargin::Standard
    }
}

impl PageMargin {
    /// Creates a uniform margin on all 4 sides.
    pub fn uniform(value: impl Into<String>) -> Self {
        let v = value.into();
        PageMargin::Custom {
            top: v.clone(),
            right: v.clone(),
            bottom: v.clone(),
            left: v,
        }
    }

    /// Creates symmetric margins with vertical (top/bottom) and horizontal (left/right).
    pub fn symmetric(vertical: impl Into<String>, horizontal: impl Into<String>) -> Self {
        let vert = vertical.into();
        let horiz = horizontal.into();
        PageMargin::Custom {
            top: vert.clone(),
            right: horiz.clone(),
            bottom: vert,
            left: horiz,
        }
    }

    /// Formats the margin as a CSS `margin` property value.
    pub fn to_css_value(&self) -> String {
        match self {
            PageMargin::Standard => "15mm 12mm 15mm 12mm".to_string(),
            PageMargin::Compact => "8mm 8mm 8mm 8mm".to_string(),
            PageMargin::Wide => "25mm 25mm 25mm 25mm".to_string(),
            PageMargin::None => "0".to_string(),
            PageMargin::Custom {
                top,
                right,
                bottom,
                left,
            } => format!("{top} {right} {bottom} {left}"),
        }
    }
}

/// Visual theme presets for print and PDF output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PrintTheme {
    /// Clean light layout with subtle slate borders and shaded code backgrounds.
    #[default]
    CleanLight,
    /// Ink-saving pure black-and-white. Zero fills, crisp solid outlines, no colored ink used.
    Monochrome,
    /// High-contrast deep black text and bold borders for maximum accessibility and readability.
    HighContrast,
    /// Preserves syntax highlighting and badge colors while ensuring clean white backgrounds.
    ColorPreserved,
    /// Minimalist typography-focused presentation with minimal borders and generous spacing.
    Minimalist,
}

/// Font size scaling for printed transcripts.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum FontSizeScale {
    /// Compact 8.5pt body / 7.5pt code (fits more content per page).
    Small,
    /// Standard 10pt body / 8.5pt code (default balance).
    #[default]
    Normal,
    /// Medium 11pt body / 9.5pt code.
    Medium,
    /// Large 12pt body / 10.5pt code (for presentations or vision-impaired reading).
    Large,
    /// Custom font size in points (`pt`).
    Custom { body_pt: f32, code_pt: f32 },
}

impl FontSizeScale {
    /// Returns the body font size in points.
    pub fn body_pt(&self) -> f32 {
        match self {
            FontSizeScale::Small => 8.5,
            FontSizeScale::Normal => 10.0,
            FontSizeScale::Medium => 11.0,
            FontSizeScale::Large => 12.0,
            FontSizeScale::Custom { body_pt, .. } => *body_pt,
        }
    }

    /// Returns the code font size in points.
    pub fn code_pt(&self) -> f32 {
        match self {
            FontSizeScale::Small => 7.5,
            FontSizeScale::Normal => 8.5,
            FontSizeScale::Medium => 9.5,
            FontSizeScale::Large => 10.5,
            FontSizeScale::Custom { code_pt, .. } => *code_pt,
        }
    }
}

/// Font family stack for printed documents.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum PrintFontFamily {
    /// Modern system sans-serif font stack (Apple-system, Segoe UI, Roboto, Helvetica, Arial).
    #[default]
    SystemSans,
    /// Elegant serif font stack (Georgia, Cambria, Times New Roman).
    SystemSerif,
    /// Modern clean sans-serif (Inter, system-ui, sans-serif).
    CleanModern,
    /// Monospace typewriter aesthetic (Cascadia Code, SFMono-Regular, Menlo, Monaco).
    Monospace,
    /// Custom CSS font-family string.
    Custom(String),
}

impl PrintFontFamily {
    /// Returns the CSS font-family string for body text.
    pub fn body_font_stack(&self) -> &str {
        match self {
            PrintFontFamily::SystemSans => {
                "-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, 'Helvetica Neue', Arial, sans-serif"
            }
            PrintFontFamily::SystemSerif => {
                "Georgia, Cambria, 'Times New Roman', Times, serif"
            }
            PrintFontFamily::CleanModern => {
                "'Inter', system-ui, -apple-system, BlinkMacSystemFont, sans-serif"
            }
            PrintFontFamily::Monospace => {
                "ui-monospace, 'Cascadia Code', 'Source Code Pro', Menlo, Monaco, Consolas, monospace"
            }
            PrintFontFamily::Custom(stack) => stack.as_str(),
        }
    }

    /// Returns the CSS font-family string for code blocks.
    pub fn code_font_stack(&self) -> &str {
        "ui-monospace, SFMono-Regular, 'Cascadia Code', 'Source Code Pro', Menlo, Monaco, Consolas, 'Liberation Mono', monospace"
    }
}

/// Line height settings for print typography.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum PrintLineHeight {
    /// Compact 1.3 line height.
    Tight,
    /// Standard 1.5 line height.
    #[default]
    Normal,
    /// Generous 1.7 line height.
    Relaxed,
    /// Custom line height multiplier.
    Custom(f32),
}

impl PrintLineHeight {
    /// Formats as a CSS line-height value.
    pub fn as_css_value(&self) -> String {
        match self {
            PrintLineHeight::Tight => "1.3".to_string(),
            PrintLineHeight::Normal => "1.5".to_string(),
            PrintLineHeight::Relaxed => "1.7".to_string(),
            PrintLineHeight::Custom(val) => format!("{val:.2}"),
        }
    }
}

/// Configuration for running headers printed at the top of each page.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RunningHeaderConfig {
    /// Content in top-left margin box.
    pub left: Option<String>,
    /// Content in top-center margin box.
    pub center: Option<String>,
    /// Content in top-right margin box.
    pub right: Option<String>,
}

impl RunningHeaderConfig {
    /// Creates an empty header configuration.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets left header text.
    pub fn with_left(mut self, text: impl Into<String>) -> Self {
        self.left = Some(text.into());
        self
    }

    /// Sets center header text.
    pub fn with_center(mut self, text: impl Into<String>) -> Self {
        self.center = Some(text.into());
        self
    }

    /// Sets right header text.
    pub fn with_right(mut self, text: impl Into<String>) -> Self {
        self.right = Some(text.into());
        self
    }
}

/// Configuration for running footers printed at the bottom of each page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningFooterConfig {
    /// Content in bottom-left margin box. Default: "Fusion AI Transcript".
    pub left: Option<String>,
    /// Content in bottom-center margin box.
    pub center: Option<String>,
    /// Content in bottom-right margin box. Default: "Page " counter(page) " of " counter(pages).
    pub right: Option<String>,
}

impl Default for RunningFooterConfig {
    fn default() -> Self {
        Self {
            left: Some("Fusion AI Transcript".to_string()),
            center: None,
            right: Some(r#""Page " counter(page) " of " counter(pages)"#.to_string()),
        }
    }
}

impl RunningFooterConfig {
    /// Creates a default running footer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets left footer text.
    pub fn with_left(mut self, text: impl Into<String>) -> Self {
        self.left = Some(text.into());
        self
    }

    /// Sets center footer text.
    pub fn with_center(mut self, text: impl Into<String>) -> Self {
        self.center = Some(text.into());
        self
    }

    /// Sets right footer text.
    pub fn with_right(mut self, text: impl Into<String>) -> Self {
        self.right = Some(text.into());
        self
    }
}

/// Page break policy controls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageBreakSettings {
    /// Avoid page breaks inside individual message cards. Default: true.
    pub avoid_break_inside_cards: bool,
    /// Avoid page breaks inside code blocks. Default: true.
    pub avoid_break_inside_code: bool,
    /// Avoid page breaks inside tables. Default: true.
    pub avoid_break_inside_tables: bool,
    /// Avoid page breaks inside tool invocations and results. Default: true.
    pub avoid_break_inside_tools: bool,
    /// Force a page break before each User turn. Default: false.
    pub break_before_user_turn: bool,
    /// Minimum orphan lines at bottom of page. Default: 3.
    pub orphans: u8,
    /// Minimum widow lines at top of page. Default: 3.
    pub widows: u8,
}

impl Default for PageBreakSettings {
    fn default() -> Self {
        Self {
            avoid_break_inside_cards: true,
            avoid_break_inside_code: true,
            avoid_break_inside_tables: true,
            avoid_break_inside_tools: true,
            break_before_user_turn: false,
            orphans: 3,
            widows: 3,
        }
    }
}

// ============================================================================
// PrintOptions Builder
// ============================================================================

/// Comprehensive options for tailoring print and PDF stylesheets.
#[derive(Debug, Clone, PartialEq)]
pub struct PrintOptions {
    /// Paper size. Default: `PageSize::A4`.
    pub page_size: PageSize,
    /// Paper orientation. Default: `PageOrientation::Portrait`.
    pub orientation: PageOrientation,
    /// Page margins. Default: `PageMargin::Standard`.
    pub margins: PageMargin,
    /// Print visual theme. Default: `PrintTheme::CleanLight`.
    pub theme: PrintTheme,
    /// Font size scale. Default: `FontSizeScale::Normal`.
    pub font_size: FontSizeScale,
    /// Font family. Default: `PrintFontFamily::SystemSans`.
    pub font_family: PrintFontFamily,
    /// Line height. Default: `PrintLineHeight::Normal`.
    pub line_height: PrintLineHeight,
    /// Running header configuration.
    pub running_header: Option<RunningHeaderConfig>,
    /// Running footer configuration.
    pub running_footer: Option<RunningFooterConfig>,
    /// Page break configuration.
    pub page_breaks: PageBreakSettings,
    /// Force collapsible `<details>` elements open during print. Default: true.
    pub expand_details: bool,
    /// Hide interactive non-printable elements (`.toolbar`, `.copy-btn`, etc.). Default: true.
    pub hide_interactive: bool,
    /// Print link URLs next to anchor tags. Default: false.
    pub show_urls: bool,
    /// Preserve syntax highlighting colors in code blocks. Default: true.
    pub preserve_syntax_colors: bool,
    /// Optional custom CSS injected at the end of the print stylesheet.
    pub custom_css: Option<String>,
}

impl Default for PrintOptions {
    fn default() -> Self {
        Self {
            page_size: PageSize::A4,
            orientation: PageOrientation::Portrait,
            margins: PageMargin::Standard,
            theme: PrintTheme::CleanLight,
            font_size: FontSizeScale::Normal,
            font_family: PrintFontFamily::SystemSans,
            line_height: PrintLineHeight::Normal,
            running_header: None,
            running_footer: Some(RunningFooterConfig::default()),
            page_breaks: PageBreakSettings::default(),
            expand_details: true,
            hide_interactive: true,
            show_urls: false,
            preserve_syntax_colors: true,
            custom_css: None,
        }
    }
}

impl PrintOptions {
    /// Creates a new `PrintOptions` builder with standard defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets paper size.
    pub fn with_page_size(mut self, size: PageSize) -> Self {
        self.page_size = size;
        self
    }

    /// Sets paper orientation.
    pub fn with_orientation(mut self, orientation: PageOrientation) -> Self {
        self.orientation = orientation;
        self
    }

    /// Sets page margins.
    pub fn with_margins(mut self, margins: PageMargin) -> Self {
        self.margins = margins;
        self
    }

    /// Sets visual print theme.
    pub fn with_theme(mut self, theme: PrintTheme) -> Self {
        self.theme = theme;
        self
    }

    /// Sets font size scale.
    pub fn with_font_size(mut self, size: FontSizeScale) -> Self {
        self.font_size = size;
        self
    }

    /// Sets font family.
    pub fn with_font_family(mut self, family: PrintFontFamily) -> Self {
        self.font_family = family;
        self
    }

    /// Sets line height.
    pub fn with_line_height(mut self, height: PrintLineHeight) -> Self {
        self.line_height = height;
        self
    }

    /// Sets running header configuration.
    pub fn with_running_header(mut self, header: RunningHeaderConfig) -> Self {
        self.running_header = Some(header);
        self
    }

    /// Sets running footer configuration.
    pub fn with_running_footer(mut self, footer: RunningFooterConfig) -> Self {
        self.running_footer = Some(footer);
        self
    }

    /// Sets page break settings.
    pub fn with_page_breaks(mut self, breaks: PageBreakSettings) -> Self {
        self.page_breaks = breaks;
        self
    }

    /// Toggles automatic expansion of `<details>` tags during print.
    pub fn with_expand_details(mut self, expand: bool) -> Self {
        self.expand_details = expand;
        self
    }

    /// Toggles hiding of interactive buttons and toolbars.
    pub fn with_hide_interactive(mut self, hide: bool) -> Self {
        self.hide_interactive = hide;
        self
    }

    /// Toggles printing of full URL targets alongside links.
    pub fn with_show_urls(mut self, show: bool) -> Self {
        self.show_urls = show;
        self
    }

    /// Toggles syntax highlighting preservation in code blocks.
    pub fn with_preserve_syntax_colors(mut self, preserve: bool) -> Self {
        self.preserve_syntax_colors = preserve;
        self
    }

    /// Injects custom CSS rules at the end of the generated print styles.
    pub fn with_custom_css(mut self, css: impl Into<String>) -> Self {
        self.custom_css = Some(css.into());
        self
    }

    /// Preset for standard US Letter format.
    pub fn letter() -> Self {
        Self::default().with_page_size(PageSize::Letter)
    }

    /// Preset for ink-saving pure monochrome printing.
    pub fn monochrome() -> Self {
        Self::default()
            .with_theme(PrintTheme::Monochrome)
            .with_preserve_syntax_colors(false)
    }

    /// Preset for compact layout (small fonts and margins to minimize page count).
    pub fn compact() -> Self {
        Self::default()
            .with_margins(PageMargin::Compact)
            .with_font_size(FontSizeScale::Small)
            .with_line_height(PrintLineHeight::Tight)
    }
}

// ============================================================================
// PdfOptions Builder
// ============================================================================

/// Specialized configuration for headless PDF rendering engines
/// (e.g. Chrome Headless, WeasyPrint, PrinceXML, Typst, wkhtmltopdf).
#[derive(Debug, Clone, PartialEq)]
pub struct PdfOptions {
    /// Underlying print options (margins, sizes, typography).
    pub print_options: PrintOptions,
    /// Target DPI resolution (standard: 300 for high-res print, 96/150 for digital screen). Default: 300.
    pub dpi: u32,
    /// PDF document title metadata.
    pub title: Option<String>,
    /// PDF document author metadata.
    pub author: Option<String>,
    /// PDF document subject metadata.
    pub subject: Option<String>,
    /// PDF keywords metadata.
    pub keywords: Vec<String>,
    /// Generate PDF outlines / bookmarks from headings. Default: true.
    pub generate_bookmarks: bool,
    /// Convert colors to grayscale. Default: false.
    pub grayscale: bool,
    /// Zoom scale factor. Default: 1.0.
    pub zoom: f32,
}

impl Default for PdfOptions {
    fn default() -> Self {
        Self {
            print_options: PrintOptions::default(),
            dpi: 300,
            title: None,
            author: Some("Fusion AI Assistant".to_string()),
            subject: Some("AI Session Transcript".to_string()),
            keywords: vec![
                "Fusion".to_string(),
                "Transcript".to_string(),
                "AI".to_string(),
            ],
            generate_bookmarks: true,
            grayscale: false,
            zoom: 1.0,
        }
    }
}

impl PdfOptions {
    /// Creates default PDF configuration.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets underlying print options.
    pub fn with_print_options(mut self, options: PrintOptions) -> Self {
        self.print_options = options;
        self
    }

    /// Sets target DPI.
    pub fn with_dpi(mut self, dpi: u32) -> Self {
        self.dpi = dpi;
        self
    }

    /// Sets PDF title metadata.
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Sets PDF author metadata.
    pub fn with_author(mut self, author: impl Into<String>) -> Self {
        self.author = Some(author.into());
        self
    }

    /// Sets PDF subject metadata.
    pub fn with_subject(mut self, subject: impl Into<String>) -> Self {
        self.subject = Some(subject.into());
        self
    }

    /// Adds a keyword for PDF metadata.
    pub fn with_keyword(mut self, keyword: impl Into<String>) -> Self {
        self.keywords.push(keyword.into());
        self
    }

    /// Toggles grayscale output.
    pub fn with_grayscale(mut self, grayscale: bool) -> Self {
        self.grayscale = grayscale;
        if grayscale {
            self.print_options.theme = PrintTheme::Monochrome;
            self.print_options.preserve_syntax_colors = false;
        }
        self
    }

    /// Sets zoom factor.
    pub fn with_zoom(mut self, zoom: f32) -> Self {
        self.zoom = zoom;
        self
    }
}

// ============================================================================
// CSS Generators
// ============================================================================

/// Generates the `@page` CSS rule based on the provided `PrintOptions`.
pub fn generate_page_css(options: &PrintOptions) -> String {
    let mut css = String::with_capacity(512);
    let size_val = options.page_size.to_css_value(options.orientation);
    let margin_val = options.margins.to_css_value();

    css.push_str("@page {\n");
    let _ = writeln!(css, "  size: {size_val};");
    let _ = writeln!(css, "  margin: {margin_val};");

    // Running Header margin boxes
    if let Some(header) = &options.running_header {
        if let Some(left) = &header.left {
            let _ = writeln!(css, "  @top-left {{ content: {left}; font-size: 8pt; color: #64748b; font-family: sans-serif; }}");
        }
        if let Some(center) = &header.center {
            let _ = writeln!(css, "  @top-center {{ content: {center}; font-size: 8pt; color: #64748b; font-family: sans-serif; }}");
        }
        if let Some(right) = &header.right {
            let _ = writeln!(css, "  @top-right {{ content: {right}; font-size: 8pt; color: #64748b; font-family: sans-serif; }}");
        }
    }

    // Running Footer margin boxes
    if let Some(footer) = &options.running_footer {
        if let Some(left) = &footer.left {
            let _ = writeln!(css, "  @bottom-left {{ content: {left}; font-size: 8pt; color: #94a3b8; font-family: sans-serif; }}");
        }
        if let Some(center) = &footer.center {
            let _ = writeln!(css, "  @bottom-center {{ content: {center}; font-size: 8pt; color: #94a3b8; font-family: sans-serif; }}");
        }
        if let Some(right) = &footer.right {
            let _ = writeln!(css, "  @bottom-right {{ content: {right}; font-size: 8pt; color: #64748b; font-family: sans-serif; }}");
        }
    }

    css.push_str("}\n");
    css
}

/// Generates print CSS rules customized with the given `PrintOptions`.
///
/// Output contains all rules ready to be placed inside an `@media print { ... }` block
/// or used directly in a standalone print stylesheet.
pub fn generate_print_css_rules(options: &PrintOptions) -> String {
    let mut css = String::with_capacity(4096);

    let body_pt = options.font_size.body_pt();
    let code_pt = options.font_size.code_pt();
    let line_height = options.line_height.as_css_value();
    let font_family = options.font_family.body_font_stack();
    let code_family = options.font_family.code_font_stack();

    // Universal resets
    css.push_str(
        r#"  *,
  *::before,
  *::after {
    box-sizing: border-box !important;
    text-shadow: none !important;
    box-shadow: none !important;
    -webkit-print-color-adjust: exact !important;
    print-color-adjust: exact !important;
  }
"#,
    );

    // Body styling
    let (bg_color, text_color, heading_color) = match options.theme {
        PrintTheme::CleanLight | PrintTheme::ColorPreserved => ("#ffffff", "#111827", "#0f172a"),
        PrintTheme::Monochrome => ("#ffffff", "#000000", "#000000"),
        PrintTheme::HighContrast => ("#ffffff", "#000000", "#000000"),
        PrintTheme::Minimalist => ("#ffffff", "#1e293b", "#0f172a"),
    };

    let _ = writeln!(
        css,
        r#"  html,
  body {{
    background: {bg_color} !important;
    color: {text_color} !important;
    font-family: {font_family} !important;
    font-size: {body_pt:.1}pt !important;
    line-height: {line_height} !important;
    padding: 0 !important;
    margin: 0 !important;
    width: 100% !important;
  }}"#
    );

    // Hide interactive elements
    if options.hide_interactive {
        css.push_str(
            r#"
  .header-actions,
  .toolbar,
  .filter-group,
  .search-box,
  .copy-btn,
  .copy-code-btn,
  .theme-toggle,
  .audio-player,
  .export-dropdown,
  .floating-action-button,
  .no-print,
  [data-no-print="true"] {
    display: none !important;
    visibility: hidden !important;
  }
"#,
        );
    }

    // Expand details
    if options.expand_details {
        css.push_str(
            r#"
  details {
    display: block !important;
  }
  details > summary {
    list-style: none !important;
    cursor: default !important;
    font-weight: 600 !important;
    padding-bottom: 4pt !important;
  }
  details > summary::-webkit-details-marker {
    display: none !important;
  }
"#,
        );
    }

    // Headings & typography
    let h1_pt = body_pt * 1.6;
    let h2_pt = body_pt * 1.3;
    let h3_pt = body_pt * 1.1;
    let orphans = options.page_breaks.orphans;
    let widows = options.page_breaks.widows;

    let _ = writeln!(
        css,
        r#"
  h1, h2, h3, h4, h5, h6 {{
    color: {heading_color} !important;
    break-after: avoid !important;
    page-break-after: avoid !important;
    font-weight: 700 !important;
  }}
  h1 {{ font-size: {h1_pt:.1}pt !important; margin-top: 0 !important; margin-bottom: 8pt !important; }}
  h2 {{ font-size: {h2_pt:.1}pt !important; margin-top: 14pt !important; margin-bottom: 6pt !important; }}
  h3 {{ font-size: {h3_pt:.1}pt !important; margin-top: 10pt !important; margin-bottom: 4pt !important; }}

  p, li, blockquote {{
    orphans: {orphans} !important;
    widows: {widows} !important;
  }}"#
    );

    // App container
    css.push_str(
        r#"
  .app-container {
    max-width: 100% !important;
    margin: 0 !important;
    padding: 0 !important;
    border: none !important;
    box-shadow: none !important;
  }
"#,
    );

    // Header card
    let header_border = match options.theme {
        PrintTheme::CleanLight | PrintTheme::ColorPreserved => "1.5pt solid #cbd5e1",
        PrintTheme::Monochrome => "1.5pt solid #000000",
        PrintTheme::HighContrast => "2pt solid #000000",
        PrintTheme::Minimalist => "none",
    };

    let _ = writeln!(
        css,
        r#"  .app-header {{
    background: #ffffff !important;
    border: {header_border} !important;
    border-radius: 4pt !important;
    padding: 10pt 12pt !important;
    margin-bottom: 12pt !important;
    page-break-inside: avoid !important;
    break-inside: avoid !important;
  }}
  .session-title {{
    font-size: {h1_pt:.1}pt !important;
    font-weight: 700 !important;
    color: {heading_color} !important;
    margin-bottom: 4pt !important;
  }}
  .metadata-bar,
  .stats-bar {{
    font-size: {code_pt:.1}pt !important;
    color: #475569 !important;
    display: flex !important;
    flex-wrap: wrap !important;
    gap: 8pt !important;
    border-top: 0.5pt solid #e2e8f0 !important;
    padding-top: 6pt !important;
    margin-top: 6pt !important;
  }}"#
    );

    // Message cards & roles
    let (card_border, user_border, asst_border, sys_border, tool_border) = match options.theme {
        PrintTheme::CleanLight => (
            "1pt solid #e2e8f0",
            "3.5pt solid #3b82f6",
            "3.5pt solid #10b981",
            "3.5pt solid #64748b",
            "3.5pt solid #f59e0b",
        ),
        PrintTheme::ColorPreserved => (
            "1pt solid #cbd5e1",
            "3.5pt solid #2563eb",
            "3.5pt solid #059669",
            "3.5pt solid #475569",
            "3.5pt solid #d97706",
        ),
        PrintTheme::Monochrome => (
            "1pt solid #000000",
            "3pt solid #000000",
            "1.5pt solid #000000",
            "1pt dashed #000000",
            "1pt double #000000",
        ),
        PrintTheme::HighContrast => (
            "1.5pt solid #000000",
            "4pt solid #000000",
            "4pt solid #000000",
            "2pt solid #000000",
            "2pt solid #000000",
        ),
        PrintTheme::Minimalist => (
            "none",
            "2pt solid #cbd5e1",
            "2pt solid #e2e8f0",
            "1pt solid #f1f5f9",
            "1pt solid #fef3c7",
        ),
    };

    let user_break = if options.page_breaks.break_before_user_turn {
        "page-break-before: always !important;\n    break-before: page !important;"
    } else {
        ""
    };

    let card_break_avoid = if options.page_breaks.avoid_break_inside_cards {
        "page-break-inside: avoid !important;\n    break-inside: avoid !important;"
    } else {
        ""
    };

    let _ = writeln!(
        css,
        r#"  .timeline {{
    display: block !important;
  }}
  .message-card {{
    background: #ffffff !important;
    border: {card_border} !important;
    border-radius: 4pt !important;
    margin-bottom: 10pt !important;
    padding: 8pt 10pt !important;
    {card_break_avoid}
  }}
  .message-card.role-user {{
    border-left: {user_border} !important;
    background: #f8fafc !important;
    {user_break}
  }}
  .message-card.role-assistant {{
    border-left: {asst_border} !important;
    background: #ffffff !important;
  }}
  .message-card.role-system {{
    border-left: {sys_border} !important;
    background: #f1f5f9 !important;
  }}
  .message-card.role-tool {{
    border-left: {tool_border} !important;
    background: #fffbeb !important;
  }}
  .message-header {{
    display: flex !important;
    justify-content: space-between !important;
    align-items: center !important;
    border-bottom: 0.5pt solid #e2e8f0 !important;
    padding-bottom: 4pt !important;
    margin-bottom: 6pt !important;
    font-size: {code_pt:.1}pt !important;
    color: #475569 !important;
  }}
  .role-badge {{
    font-weight: 700 !important;
    font-size: 8pt !important;
    text-transform: uppercase !important;
    padding: 1.5pt 4pt !important;
    border-radius: 2pt !important;
    border: 0.5pt solid #cbd5e1 !important;
  }}"#
    );

    // Thinking / DeepSeek reasoning blocks
    css.push_str(
        r#"  .thinking-block,
  .thought-content,
  blockquote.think-block {
    background: #f8fafc !important;
    border-left: 2.5pt solid #94a3b8 !important;
    color: #334155 !important;
    font-style: italic !important;
    padding: 6pt 8pt !important;
    margin: 6pt 0 !important;
    border-radius: 0 3pt 3pt 0 !important;
    font-size: 9pt !important;
    page-break-inside: avoid !important;
    break-inside: avoid !important;
  }
"#,
    );

    // Tool execution blocks
    let tool_break_avoid = if options.page_breaks.avoid_break_inside_tools {
        "page-break-inside: avoid !important;\n    break-inside: avoid !important;"
    } else {
        ""
    };

    let _ = writeln!(
        css,
        r#"  .tool-call,
  .tool-result {{
    background: #f8fafc !important;
    border: 1pt solid #cbd5e1 !important;
    border-radius: 3pt !important;
    padding: 6pt 8pt !important;
    margin: 6pt 0 !important;
    font-size: {code_pt:.1}pt !important;
    {tool_break_avoid}
  }}
  .tool-name {{
    font-family: {code_family} !important;
    font-weight: 700 !important;
    color: {heading_color} !important;
  }}"#
    );

    // Code blocks & Syntax highlighting
    let code_break_avoid = if options.page_breaks.avoid_break_inside_code {
        "page-break-inside: avoid !important;\n    break-inside: avoid !important;"
    } else {
        ""
    };

    let code_bg = match options.theme {
        PrintTheme::CleanLight | PrintTheme::ColorPreserved => "#f8fafc",
        PrintTheme::Monochrome => "#ffffff",
        PrintTheme::HighContrast => "#f1f5f9",
        PrintTheme::Minimalist => "#f8fafc",
    };

    let code_border = match options.theme {
        PrintTheme::CleanLight | PrintTheme::ColorPreserved => "1pt solid #cbd5e1",
        PrintTheme::Monochrome => "1pt solid #000000",
        PrintTheme::HighContrast => "1.5pt solid #000000",
        PrintTheme::Minimalist => "1pt solid #e2e8f0",
    };

    let _ = writeln!(
        css,
        r#"  pre,
  code {{
    font-family: {code_family} !important;
    font-size: {code_pt:.1}pt !important;
    tab-size: 2 !important;
  }}
  pre {{
    background: {code_bg} !important;
    color: {text_color} !important;
    border: {code_border} !important;
    border-radius: 3pt !important;
    padding: 6pt 8pt !important;
    margin: 6pt 0 !important;
    white-space: pre-wrap !important;
    word-break: break-word !important;
    overflow-wrap: break-word !important;
    {code_break_avoid}
  }}
  code:not(pre code) {{
    background: #f1f5f9 !important;
    color: {text_color} !important;
    padding: 1pt 3pt !important;
    border-radius: 2pt !important;
    border: 0.5pt solid #e2e8f0 !important;
    font-size: {code_pt:.1}pt !important;
  }}"#
    );

    // Monochrome code syntax override if preserve_syntax_colors is false
    if !options.preserve_syntax_colors || options.theme == PrintTheme::Monochrome {
        css.push_str(
            r#"  .hl-kw, .hl-fn, .hl-str, .hl-num, .hl-type, .hl-comm, .hl-tag, .hl-attr {
    color: #000000 !important;
    font-weight: normal !important;
  }
  .hl-kw, .hl-tag {
    font-weight: 700 !important;
  }
  .hl-comm {
    font-style: italic !important;
    color: #4b5563 !important;
  }
"#,
        );
    }

    // Tables
    let table_break_avoid = if options.page_breaks.avoid_break_inside_tables {
        "page-break-inside: avoid !important;\n    break-inside: avoid !important;"
    } else {
        ""
    };

    let _ = writeln!(
        css,
        r#"  table {{
    width: 100% !important;
    border-collapse: collapse !important;
    margin: 8pt 0 !important;
    font-size: {code_pt:.1}pt !important;
    {table_break_avoid}
  }}
  thead {{
    display: table-header-group !important;
  }}
  tr {{
    page-break-inside: avoid !important;
    break-inside: avoid !important;
  }}
  th, td {{
    border: 0.75pt solid #cbd5e1 !important;
    padding: 4pt 6pt !important;
    text-align: left !important;
  }}
  th {{
    background: #f1f5f9 !important;
    font-weight: 700 !important;
    color: {heading_color} !important;
  }}
  tbody tr:nth-child(even) {{
    background: #f8fafc !important;
  }}"#
    );

    // Blockquotes
    css.push_str(
        r#"  blockquote {
    border-left: 3pt solid #cbd5e1 !important;
    margin: 6pt 0 !important;
    padding: 4pt 8pt !important;
    color: #334155 !important;
    background: #f8fafc !important;
    page-break-inside: avoid !important;
    break-inside: avoid !important;
  }
"#,
    );

    // Links & optional URL printing
    if options.show_urls {
        css.push_str(
            r#"  a {
    color: #0f172a !important;
    text-decoration: underline !important;
  }
  a[href^="http"]:after,
  a[href^="https"]:after {
    content: " (" attr(href) ")" !important;
    font-size: 80% !important;
    color: #64748b !important;
  }
"#,
        );
    } else {
        css.push_str(
            r#"  a {
    color: #0f172a !important;
    text-decoration: underline !important;
  }
"#,
        );
    }

    // App Footer
    let _ = writeln!(
        css,
        r#"  .app-footer {{
    border-top: 1pt solid #cbd5e1 !important;
    padding-top: 8pt !important;
    margin-top: 14pt !important;
    font-size: {code_pt:.1}pt !important;
    color: #64748b !important;
    text-align: center !important;
    page-break-inside: avoid !important;
    break-inside: avoid !important;
  }}"#
    );

    // Custom CSS injection
    if let Some(custom) = &options.custom_css {
        css.push_str("\n  /* Custom user print overrides */\n  ");
        css.push_str(custom);
        css.push('\n');
    }

    css
}

/// Generates a complete `@media print { ... }` block along with `@page` definitions.
pub fn generate_print_css(options: &PrintOptions) -> String {
    let mut css = String::with_capacity(4096);
    css.push_str(&generate_page_css(options));
    css.push_str("\n@media print {\n");
    css.push_str(&generate_print_css_rules(options));
    css.push_str("}\n");
    css
}

/// Generates a standalone print stylesheet without an enclosing `@media print` query.
///
/// Useful for dedicated PDF rendering engines (WeasyPrint, Prince, wkhtmltopdf) where
/// the document is exclusively compiled into a printable PDF.
pub fn generate_standalone_print_stylesheet(options: &PrintOptions) -> String {
    let mut css = String::with_capacity(4096);
    css.push_str(&generate_page_css(options));
    css.push('\n');
    css.push_str(&generate_print_css_rules(options));
    css
}

/// Generates specialized CSS for PDF engines from a `PdfOptions` configuration.
pub fn generate_pdf_css(options: &PdfOptions) -> String {
    let mut css = String::with_capacity(4096);
    css.push_str(&generate_standalone_print_stylesheet(
        &options.print_options,
    ));

    if options.grayscale {
        css.push_str(
            "\n/* PDF Grayscale Filter */\nhtml { filter: grayscale(100%) !important; }\n",
        );
    }

    if (options.zoom - 1.0).abs() > f32::EPSILON {
        let _ = writeln!(
            css,
            "\n/* PDF Zoom Scale */\nbody {{ zoom: {:.2} !important; }}",
            options.zoom
        );
    }

    css
}

// ============================================================================
// HTML Injection & Transformation Helpers
// ============================================================================

/// Injects or replaces print CSS within an existing HTML document string.
///
/// If an existing `<style>` tag is found, the print styles are appended to it.
/// If no style tag is present, a new `<style>` element is inserted before `</head>` or at the top.
pub fn inject_print_css(html: &str, options: &PrintOptions) -> String {
    let print_css = generate_print_css(options);

    // Look for </style> to append before closing style tag
    if let Some(pos) = html.rfind("</style>") {
        let mut result = String::with_capacity(html.len() + print_css.len() + 32);
        result.push_str(&html[..pos]);
        result.push_str("\n/* Injected Print Stylesheet */\n");
        result.push_str(&print_css);
        result.push_str(&html[pos..]);
        return result;
    }

    // Look for </head> to insert a new <style> tag
    if let Some(pos) = html.find("</head>") {
        let mut result = String::with_capacity(html.len() + print_css.len() + 64);
        result.push_str(&html[..pos]);
        result.push_str("  <style>\n");
        result.push_str(&print_css);
        result.push_str("  </style>\n");
        result.push_str(&html[pos..]);
        return result;
    }

    // Fallback: prepend style tag
    format!("<style>\n{print_css}</style>\n{html}")
}

/// Optimizes an exported HTML session transcript specifically for PDF conversion.
///
/// Performs the following enhancements:
/// 1. Injects PDF-specialized print styles.
/// 2. Expands all `<details>` tags by adding `open=""` so collapsible sections (tool calls,
///    reasoning blocks) are visible in static PDF renders.
/// 3. Injects PDF metadata `<meta>` tags (title, author, subject, keywords).
pub fn optimize_for_pdf(html: &str, options: &PdfOptions) -> String {
    // 1. Force open on <details>
    let mut modified = html.to_string();
    if options.print_options.expand_details {
        modified = modified.replace("<details>", "<details open>");
    }

    // 2. Metadata tags
    let mut meta_tags = String::new();
    if let Some(title) = &options.title {
        let _ = writeln!(
            meta_tags,
            r#"  <meta name="title" content="{}">"#,
            escape_attr(title)
        );
    }
    if let Some(author) = &options.author {
        let _ = writeln!(
            meta_tags,
            r#"  <meta name="author" content="{}">"#,
            escape_attr(author)
        );
    }
    if let Some(subject) = &options.subject {
        let _ = writeln!(
            meta_tags,
            r#"  <meta name="subject" content="{}">"#,
            escape_attr(subject)
        );
    }
    if !options.keywords.is_empty() {
        let kw_str = options.keywords.join(", ");
        let _ = writeln!(
            meta_tags,
            r#"  <meta name="keywords" content="{}">"#,
            escape_attr(&kw_str)
        );
    }

    if !meta_tags.is_empty() {
        if let Some(pos) = modified.find("<head>") {
            let insert_pos = pos + 6;
            let mut with_meta = String::with_capacity(modified.len() + meta_tags.len() + 8);
            with_meta.push_str(&modified[..insert_pos]);
            with_meta.push('\n');
            with_meta.push_str(&meta_tags);
            with_meta.push_str(&modified[insert_pos..]);
            modified = with_meta;
        }
    }

    // 3. Inject PDF Styles
    let pdf_css = generate_pdf_css(options);
    if let Some(pos) = modified.rfind("</style>") {
        let mut result = String::with_capacity(modified.len() + pdf_css.len() + 32);
        result.push_str(&modified[..pos]);
        result.push_str("\n/* PDF-Specific Print Styles */\n");
        result.push_str(&pdf_css);
        result.push_str(&modified[pos..]);
        result
    } else if let Some(pos) = modified.find("</head>") {
        let mut result = String::with_capacity(modified.len() + pdf_css.len() + 64);
        result.push_str(&modified[..pos]);
        result.push_str("  <style>\n");
        result.push_str(&pdf_css);
        result.push_str("  </style>\n");
        result.push_str(&modified[pos..]);
        result
    } else {
        format!("<style>\n{pdf_css}</style>\n{modified}")
    }
}

/// Helper to escape attribute values in HTML meta tags.
fn escape_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_print_css_contains_core_rules() {
        assert!(DEFAULT_PRINT_CSS.contains("@media print"));
        assert!(DEFAULT_PRINT_CSS.contains(".header-actions"));
        assert!(DEFAULT_PRINT_CSS.contains(".toolbar"));
        assert!(DEFAULT_PRINT_CSS.contains("break-inside: avoid"));
        assert!(DEFAULT_PRINT_CSS.contains("details"));
        assert!(DEFAULT_PRINT_CSS.contains(".message-card"));
        assert!(DEFAULT_PRINT_CSS.contains("pre,"));
    }

    #[test]
    fn test_default_page_css() {
        assert!(DEFAULT_PAGE_CSS.contains("@page"));
        assert!(DEFAULT_PAGE_CSS.contains("size: A4 portrait"));
        assert!(DEFAULT_PAGE_CSS.contains("counter(page)"));
    }

    #[test]
    fn test_page_sizes() {
        let a4 = PageSize::A4.to_css_value(PageOrientation::Portrait);
        assert_eq!(a4, "A4 portrait");

        let letter_land = PageSize::Letter.to_css_value(PageOrientation::Landscape);
        assert_eq!(letter_land, "letter landscape");

        let legal = PageSize::Legal.to_css_value(PageOrientation::Portrait);
        assert_eq!(legal, "legal portrait");

        let custom = PageSize::Custom {
            width: "150mm".to_string(),
            height: "200mm".to_string(),
        };
        assert_eq!(
            custom.to_css_value(PageOrientation::Portrait),
            "150mm 200mm"
        );
        assert_eq!(
            custom.to_css_value(PageOrientation::Landscape),
            "200mm 150mm"
        );

        assert!(PageSize::A4.dimensions_mm().is_some());
        assert_eq!(PageSize::A4.dimensions_mm().unwrap(), (210.0, 297.0));
    }

    #[test]
    fn test_page_margins() {
        assert_eq!(PageMargin::Standard.to_css_value(), "15mm 12mm 15mm 12mm");
        assert_eq!(PageMargin::Compact.to_css_value(), "8mm 8mm 8mm 8mm");
        assert_eq!(PageMargin::Wide.to_css_value(), "25mm 25mm 25mm 25mm");
        assert_eq!(PageMargin::None.to_css_value(), "0");

        let uniform = PageMargin::uniform("10mm");
        assert_eq!(uniform.to_css_value(), "10mm 10mm 10mm 10mm");

        let sym = PageMargin::symmetric("20mm", "15mm");
        assert_eq!(sym.to_css_value(), "20mm 15mm 20mm 15mm");
    }

    #[test]
    fn test_generate_page_css() {
        let opts = PrintOptions::new()
            .with_page_size(PageSize::Letter)
            .with_orientation(PageOrientation::Landscape)
            .with_margins(PageMargin::Compact)
            .with_running_header(RunningHeaderConfig::new().with_left(r#""Fusion Transcript""#))
            .with_running_footer(RunningFooterConfig::new().with_right(r#""Page " counter(page)"#));

        let page_css = generate_page_css(&opts);
        assert!(page_css.contains("size: letter landscape;"));
        assert!(page_css.contains("margin: 8mm 8mm 8mm 8mm;"));
        assert!(page_css.contains("@top-left { content: \"Fusion Transcript\";"));
        assert!(page_css.contains("@bottom-right { content: \"Page \" counter(page);"));
    }

    #[test]
    fn test_generate_print_css_themes() {
        // CleanLight
        let light_opts = PrintOptions::new().with_theme(PrintTheme::CleanLight);
        let light_css = generate_print_css(&light_opts);
        assert!(light_css.contains("@media print"));
        assert!(light_css.contains("background: #ffffff"));

        // Monochrome
        let mono_opts = PrintOptions::monochrome();
        let mono_css = generate_print_css(&mono_opts);
        assert!(mono_css.contains("color: #000000"));
        assert!(mono_css.contains(".hl-kw, .hl-fn"));

        // HighContrast
        let hc_opts = PrintOptions::new().with_theme(PrintTheme::HighContrast);
        let hc_css = generate_print_css(&hc_opts);
        assert!(hc_css.contains("2pt solid #000000"));

        // Minimalist
        let min_opts = PrintOptions::new().with_theme(PrintTheme::Minimalist);
        let min_css = generate_print_css(&min_opts);
        assert!(min_css.contains("border: none"));
    }

    #[test]
    fn test_font_size_and_family_scaling() {
        let opts = PrintOptions::new()
            .with_font_size(FontSizeScale::Large)
            .with_font_family(PrintFontFamily::SystemSerif)
            .with_line_height(PrintLineHeight::Relaxed);

        let css = generate_print_css(&opts);
        assert!(css.contains("font-size: 12.0pt"));
        assert!(css.contains("font-family: Georgia, Cambria"));
        assert!(css.contains("line-height: 1.7"));
    }

    #[test]
    fn test_show_urls_option() {
        let without_urls = PrintOptions::new().with_show_urls(false);
        assert!(!generate_print_css(&without_urls).contains("attr(href)"));

        let with_urls = PrintOptions::new().with_show_urls(true);
        assert!(generate_print_css(&with_urls).contains("attr(href)"));
    }

    #[test]
    fn test_custom_css_injection() {
        let opts = PrintOptions::new().with_custom_css(".custom-badge { color: red !important; }");
        let css = generate_print_css(&opts);
        assert!(css.contains(".custom-badge { color: red !important; }"));
    }

    #[test]
    fn test_standalone_print_stylesheet() {
        let opts = PrintOptions::new();
        let standalone = generate_standalone_print_stylesheet(&opts);
        assert!(!standalone.contains("@media print {"));
        assert!(standalone.contains("@page {"));
        assert!(standalone.contains(".message-card"));
    }

    #[test]
    fn test_generate_pdf_css() {
        let pdf_opts = PdfOptions::new().with_grayscale(true).with_zoom(0.95);

        let css = generate_pdf_css(&pdf_opts);
        assert!(css.contains("filter: grayscale(100%)"));
        assert!(css.contains("zoom: 0.95"));
    }

    #[test]
    fn test_inject_print_css_into_html() {
        let sample_html = r#"<!DOCTYPE html>
<html>
<head>
  <title>Test</title>
  <style>
    body { font-family: sans-serif; }
  </style>
</head>
<body>
  <h1>Test Header</h1>
</body>
</html>"#;

        let opts = PrintOptions::compact();
        let injected = inject_print_css(sample_html, &opts);
        assert!(injected.contains("Injected Print Stylesheet"));
        assert!(injected.contains("size: A4 portrait"));
        assert!(injected.contains("margin: 8mm 8mm 8mm 8mm"));
    }

    #[test]
    fn test_optimize_for_pdf() {
        let html = r#"<!DOCTYPE html>
<html>
<head>
  <title>Session</title>
  <style>body { color: black; }</style>
</head>
<body>
  <details>
    <summary>Tool Execution</summary>
    <div>Tool output</div>
  </details>
</body>
</html>"#;

        let pdf_opts = PdfOptions::new()
            .with_title("Custom Session PDF")
            .with_author("Jane Developer");

        let optimized = optimize_for_pdf(html, &pdf_opts);
        assert!(optimized.contains("<details open>"));
        assert!(optimized.contains(r#"<meta name="title" content="Custom Session PDF">"#));
        assert!(optimized.contains(r#"<meta name="author" content="Jane Developer">"#));
        assert!(optimized.contains("PDF-Specific Print Styles"));
    }
}
