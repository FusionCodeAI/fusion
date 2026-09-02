//! Hexadecimal and Binary File Viewer Tool.
//!
//! High-performance, pure-Rust binary and hex inspector providing:
//! - Canonical and customizable hex dump layouts (offset, grouped hex bytes, ASCII text)
//! - Configurable byte grouping (1, 2, 4, 8, 16 bytes per group)
//! - Configurable line widths (8, 16, 24, 32, 64 bytes per line)
//! - Radix formatting (Hex, Decimal, Octal offsets; Hex, Binary, Decimal, Octal bytes)
//! - Squeeze mode (collapsing consecutive identical rows to `*`)
//! - File signature / magic number detection across 50+ formats (ELF, Mach-O, PE, WASM, SQLite, PNG, ZIP, etc.)
//! - Shannon entropy calculation (0.0 to 8.0 bits/byte) to evaluate encryption/compression
//! - Byte frequency analysis and classification (null, printable ASCII, whitespace, control, high bytes)
//! - Binary string extraction (`strings` equivalent for ASCII and UTF-16 strings)
//! - Binary pattern search with hex bytes, ASCII strings, and wildcard (`??`) support
//! - Primitive type decoding at offset (u8, i8, u16/i16, u32/i32, u64/i64, f32, f64, UTF-8, C-string, timestamp)
//! - Binary diffing / side-by-side comparison of files or byte buffers
//! - Direct input support (file paths, raw byte slices, hex strings, base64 strings)
//! - Dual output formats: human-readable formatted text or structured JSON

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::tools::file::resolve_path;
use crate::tools::types::{Tool, ToolContext};

// ===========================================================================
// Core Configuration & Enums
// ===========================================================================

/// Offset display format/radix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OffsetRadix {
    #[default]
    Hex,
    Dec,
    Oct,
}

impl OffsetRadix {
    pub fn from_str_loose(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "dec" | "decimal" | "10" | "d" => OffsetRadix::Dec,
            "oct" | "octal" | "8" | "o" => OffsetRadix::Oct,
            _ => OffsetRadix::Hex,
        }
    }
}

/// Byte display representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ByteFormat {
    #[default]
    HexLower,
    HexUpper,
    Binary,
    Decimal,
    Octal,
}

impl ByteFormat {
    pub fn from_str_loose(s: &str) -> Self {
        match s.trim().as_ref() {
            "hex_upper" | "HEX" | "hex-upper" | "uppercase" => ByteFormat::HexUpper,
            "bin" | "binary" | "2" | "b" => ByteFormat::Binary,
            "dec" | "decimal" | "10" | "d" => ByteFormat::Decimal,
            "oct" | "octal" | "8" | "o" => ByteFormat::Octal,
            _ => ByteFormat::HexLower,
        }
    }
}

/// Action mode supported by the hex viewer tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum HexAction {
    #[default]
    Dump,
    Inspect,
    Search,
    Strings,
    Decode,
    Diff,
}

impl HexAction {
    pub fn from_str_loose(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "inspect" | "analyze" | "stat" | "stats" | "info" => HexAction::Inspect,
            "search" | "find" | "pattern" => HexAction::Search,
            "strings" | "extract_strings" | "string" => HexAction::Strings,
            "decode" | "dissect" | "primitive" | "struct" => HexAction::Decode,
            "diff" | "compare" | "binary_diff" => HexAction::Diff,
            _ => HexAction::Dump,
        }
    }
}

/// Output formatting mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OutputFormat {
    #[default]
    Text,
    Json,
}

impl OutputFormat {
    pub fn from_str_loose(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "json" | "structured" => OutputFormat::Json,
            _ => OutputFormat::Text,
        }
    }
}

/// Options controlling how a hex dump is rendered.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HexDumpOptions {
    /// 0-based byte offset to start displaying from.
    pub offset: u64,
    /// Maximum number of bytes to display (None = unlimited or default limit).
    pub length: Option<usize>,
    /// Number of bytes displayed per row (default: 16).
    pub bytes_per_row: usize,
    /// Number of bytes per space-separated hex group (default: 1, e.g. 1, 2, 4, 8).
    pub group_size: usize,
    /// Offset display radix (Hex, Dec, Oct).
    pub offset_radix: OffsetRadix,
    /// Byte format (Hex lower/upper, binary, decimal, octal).
    pub byte_format: ByteFormat,
    /// Whether to show the ASCII decoded representation on the right.
    pub show_ascii: bool,
    /// Whether to show the column header.
    pub show_header: bool,
    /// Whether to show the summary footer.
    pub show_summary: bool,
    /// Whether to collapse identical consecutive lines with `*`.
    pub collapse_repeats: bool,
    /// Whether to include ANSI color codes in output text.
    pub color: bool,
}

impl Default for HexDumpOptions {
    fn default() -> Self {
        Self {
            offset: 0,
            length: Some(256),
            bytes_per_row: 16,
            group_size: 1,
            offset_radix: OffsetRadix::Hex,
            byte_format: ByteFormat::HexLower,
            show_ascii: true,
            show_header: true,
            show_summary: true,
            collapse_repeats: false,
            color: false,
        }
    }
}

// ===========================================================================
// Data Models & Analysis Structures
// ===========================================================================

/// A single row in a formatted hex dump.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HexDumpRow {
    pub offset: u64,
    pub bytes: Vec<u8>,
    pub hex_text: String,
    pub ascii_text: String,
}

/// Result of generating a hex dump.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HexDumpResult {
    pub total_size: u64,
    pub view_offset: u64,
    pub view_length: usize,
    pub rows: Vec<HexDumpRow>,
    pub file_type: Option<FileSignatureInfo>,
    pub entropy: f64,
    pub byte_stats: ByteStatistics,
    pub formatted_output: String,
}

/// File signature and format classification.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileSignatureInfo {
    pub name: String,
    pub category: String,
    pub mime_type: Option<String>,
    pub description: String,
    pub extension: String,
}

/// Byte frequency and statistical distribution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ByteStatistics {
    pub total_bytes: usize,
    pub null_bytes: usize,
    pub null_percentage: f64,
    pub printable_bytes: usize,
    pub printable_percentage: f64,
    pub whitespace_bytes: usize,
    pub whitespace_percentage: f64,
    pub control_bytes: usize,
    pub control_percentage: f64,
    pub high_bytes: usize,
    pub high_percentage: f64,
    pub entropy: f64,
    pub most_frequent: Vec<ByteFrequency>,
}

/// Frequency count for a single byte.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ByteFrequency {
    pub byte: u8,
    pub hex: String,
    pub count: usize,
    pub percentage: u32, // In basis points (0.01% = 1)
}

/// An extracted string with its offset and character encoding.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExtractedString {
    pub offset: u64,
    pub length: usize,
    pub encoding: String,
    pub value: String,
}

/// A search match inside a binary file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SearchMatch {
    pub offset: u64,
    pub length: usize,
    pub hex_preview: String,
    pub ascii_preview: String,
    pub context_hex: String,
}

/// Primitive values decoded at a specific offset.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PrimitiveDecoding {
    pub offset: u64,
    pub u8_val: u8,
    pub i8_val: i8,
    pub u16_le: Option<u16>,
    pub u16_be: Option<u16>,
    pub i16_le: Option<i16>,
    pub i16_be: Option<i16>,
    pub u32_le: Option<u32>,
    pub u32_be: Option<u32>,
    pub i32_le: Option<i32>,
    pub i32_be: Option<i32>,
    pub u64_le: Option<u64>,
    pub u64_be: Option<u64>,
    pub i64_le: Option<i64>,
    pub i64_be: Option<i64>,
    pub f32_le: Option<f32>,
    pub f32_be: Option<f32>,
    pub f64_le: Option<f64>,
    pub f64_be: Option<f64>,
    pub utf8_str: Option<String>,
    pub c_string: Option<String>,
    pub unix_time_le: Option<String>,
    pub unix_time_be: Option<String>,
}

/// Binary difference between two buffers.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BinaryDiff {
    pub file1_size: u64,
    pub file2_size: u64,
    pub differing_bytes_count: usize,
    pub differing_percentage: f64,
    pub chunks: Vec<DiffChunk>,
}

/// A contiguous chunk of differences.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiffChunk {
    pub offset: u64,
    pub length: usize,
    pub file1_bytes: Vec<u8>,
    pub file2_bytes: Vec<u8>,
    pub file1_hex: String,
    pub file2_hex: String,
    pub file1_ascii: String,
    pub file2_ascii: String,
}

// ===========================================================================
// Hex Dump Formatting Engine
// ===========================================================================

/// Formats a byte slice into a standard canonical or customized hex dump string.
pub fn format_hex_dump(data: &[u8], options: &HexDumpOptions) -> String {
    let result = hex_dump(data, options);
    result.formatted_output
}

/// Generates a full `HexDumpResult` containing both structured rows and formatted text.
pub fn hex_dump(data: &[u8], options: &HexDumpOptions) -> HexDumpResult {
    let total_size = data.len() as u64;
    let bytes_per_row = options.bytes_per_row.clamp(1, 128);
    let group_size = options.group_size.clamp(1, bytes_per_row);

    let start_offset = (options.offset as usize).min(data.len());
    let length = match options.length {
        Some(len) => len.min(data.len().saturating_sub(start_offset)),
        None => data.len().saturating_sub(start_offset),
    };

    let slice = if start_offset < data.len() {
        &data[start_offset..start_offset + length]
    } else {
        &[]
    };

    let entropy = shannon_entropy(slice);
    let byte_stats = analyze_byte_distribution(slice);
    let file_type = detect_file_type(data);

    let mut rows = Vec::new();
    let mut output = String::new();

    // 1. Optional Header
    if options.show_header {
        let header = format_header(bytes_per_row, group_size, options.offset_radix, options.show_ascii);
        output.push_str(&header);
        output.push('\n');
    }

    // 2. Generate Rows
    let mut last_chunk: Option<&[u8]> = None;
    let mut repeating = false;

    for (row_idx, chunk) in slice.chunks(bytes_per_row).enumerate() {
        let current_offset = options.offset + (row_idx * bytes_per_row) as u64;

        // Check for repeat squeezing
        if options.collapse_repeats {
            if let Some(prev) = last_chunk {
                if prev == chunk {
                    if !repeating {
                        output.push_str("*\n");
                        repeating = true;
                    }
                    continue;
                }
            }
        }
        repeating = false;
        last_chunk = Some(chunk);

        let hex_text = format_hex_bytes(chunk, bytes_per_row, group_size, options.byte_format, options.color);
        let ascii_text = if options.show_ascii {
            format_ascii_representation(chunk)
        } else {
            String::new()
        };

        let formatted_offset = format_offset(current_offset, options.offset_radix);

        let mut line = String::new();
        line.push_str(&formatted_offset);
        line.push_str("  ");
        line.push_str(&hex_text);

        if options.show_ascii {
            line.push_str("  |");
            line.push_str(&ascii_text);
            line.push('|');
        }

        output.push_str(&line);
        output.push('\n');

        rows.push(HexDumpRow {
            offset: current_offset,
            bytes: chunk.to_vec(),
            hex_text,
            ascii_text,
        });
    }

    // If slice was empty
    if slice.is_empty() {
        output.push_str("(empty or offset beyond file size)\n");
    }

    // 3. Optional Summary Footer
    if options.show_summary {
        let summary = format_summary_footer(
            total_size,
            options.offset,
            length,
            file_type.as_ref(),
            entropy,
            &byte_stats,
        );
        output.push_str(&summary);
    }

    HexDumpResult {
        total_size,
        view_offset: options.offset,
        view_length: length,
        rows,
        file_type,
        entropy,
        byte_stats,
        formatted_output: output,
    }
}

/// Formats the header row showing byte offsets and column headers.
fn format_header(bytes_per_row: usize, group_size: usize, radix: OffsetRadix, show_ascii: bool) -> String {
    let mut header = match radix {
        OffsetRadix::Hex => "Offset(h) ".to_string(),
        OffsetRadix::Dec => "Offset(d) ".to_string(),
        OffsetRadix::Oct => "Offset(o) ".to_string(),
    };

    header.push(' ');

    for i in 0..bytes_per_row {
        if i > 0 && i % group_size == 0 {
            header.push(' ');
        }
        if bytes_per_row == 16 && i == 8 && group_size == 1 {
            header.push(' '); // Midpoint spacing for 16-byte standard layout
        }

        match radix {
            OffsetRadix::Hex => header.push_str(&format!("{:02X}", i % 256)),
            OffsetRadix::Dec => header.push_str(&format!("{:02}", i % 100)),
            OffsetRadix::Oct => header.push_str(&format!("{:02o}", i % 64)),
        }
    }

    if show_ascii {
        let hex_field_width = compute_hex_field_width(bytes_per_row, group_size);
        let header_used = header.len().saturating_sub(11);
        let padding = hex_field_width.saturating_sub(header_used);
        for _ in 0..padding {
            header.push(' ');
        }
        header.push_str("  Decoded Text");
    }

    header
}

/// Computes the width of the hex bytes column for proper alignment.
fn compute_hex_field_width(bytes_per_row: usize, group_size: usize) -> usize {
    let mut width = bytes_per_row * 2; // 2 chars per byte
    let num_spaces = if group_size > 0 {
        (bytes_per_row.saturating_sub(1)) / group_size
    } else {
        0
    };
    width += num_spaces;
    if bytes_per_row == 16 && group_size == 1 {
        width += 1; // Midpoint space
    }
    width
}

/// Formats an offset value according to radix.
fn format_offset(offset: u64, radix: OffsetRadix) -> String {
    match radix {
        OffsetRadix::Hex => {
            if offset > 0xFFFF_FFFF {
                format!("{:016X}", offset)
            } else {
                format!("{:08X}", offset)
            }
        }
        OffsetRadix::Dec => format!("{:08}", offset),
        OffsetRadix::Oct => format!("{:08o}", offset),
    }
}

/// Formats a chunk of bytes as hex text with alignment, grouping, and optional ANSI color.
fn format_hex_bytes(
    chunk: &[u8],
    bytes_per_row: usize,
    group_size: usize,
    format: ByteFormat,
    color: bool,
) -> String {
    let mut out = String::new();
    let mut count = 0;

    for (i, &b) in chunk.iter().enumerate() {
        if i > 0 && i % group_size == 0 {
            out.push(' ');
            count += 1;
        }
        if bytes_per_row == 16 && i == 8 && group_size == 1 {
            out.push(' ');
            count += 1;
        }

        let formatted_byte = match format {
            ByteFormat::HexLower => format!("{:02x}", b),
            ByteFormat::HexUpper => format!("{:02X}", b),
            ByteFormat::Binary => format!("{:08b}", b),
            ByteFormat::Decimal => format!("{:03}", b),
            ByteFormat::Octal => format!("{:03o}", b),
        };

        if color {
            let colored = colorize_byte(b, &formatted_byte);
            out.push_str(&colored);
        } else {
            out.push_str(&formatted_byte);
        }

        count += formatted_byte.len();
    }

    // Pad remaining space if chunk is shorter than bytes_per_row (e.g. last line)
    if chunk.len() < bytes_per_row {
        let expected_total_width = match format {
            ByteFormat::HexLower | ByteFormat::HexUpper => {
                compute_hex_field_width(bytes_per_row, group_size)
            }
            ByteFormat::Binary => {
                bytes_per_row * 8 + (bytes_per_row.saturating_sub(1) / group_size)
            }
            ByteFormat::Decimal | ByteFormat::Octal => {
                bytes_per_row * 3 + (bytes_per_row.saturating_sub(1) / group_size)
            }
        };

        // Note: count tracks raw character count without ANSI escapes
        let raw_char_count = match format {
            ByteFormat::HexLower | ByteFormat::HexUpper => {
                chunk.len() * 2
                    + (if group_size > 0 {
                        chunk.len().saturating_sub(1) / group_size
                    } else {
                        0
                    })
                    + (if bytes_per_row == 16 && chunk.len() > 8 && group_size == 1 {
                        1
                    } else {
                        0
                    })
            }
            ByteFormat::Binary => {
                chunk.len() * 8
                    + (if group_size > 0 {
                        chunk.len().saturating_sub(1) / group_size
                    } else {
                        0
                    })
            }
            ByteFormat::Decimal | ByteFormat::Octal => {
                chunk.len() * 3
                    + (if group_size > 0 {
                        chunk.len().saturating_sub(1) / group_size
                    } else {
                        0
                    })
            }
        };

        let pad_amount = expected_total_width.saturating_sub(raw_char_count);
        for _ in 0..pad_amount {
            out.push(' ');
        }
    }

    out
}

/// Formats ASCII representation: printable ASCII chars are preserved, all others converted to `.`.
fn format_ascii_representation(chunk: &[u8]) -> String {
    let mut out = String::with_capacity(chunk.len());
    for &b in chunk {
        if (0x20..=0x7E).contains(&b) {
            out.push(b as char);
        } else {
            out.push('.');
        }
    }
    out
}

/// Applies ANSI color codes to a byte based on its category.
fn colorize_byte(b: u8, text: &str) -> String {
    if b == 0x00 {
        // Dim / Dark Gray for null bytes
        format!("\x1b[90m{}\x1b[0m", text)
    } else if (0x20..=0x7E).contains(&b) {
        // Bright Green for printable ASCII
        format!("\x1b[32m{}\x1b[0m", text)
    } else if b == 0x09 || b == 0x0A || b == 0x0D || b == 0x20 {
        // Cyan for whitespace
        format!("\x1b[36m{}\x1b[0m", text)
    } else if b < 0x20 {
        // Yellow for control characters
        format!("\x1b[33m{}\x1b[0m", text)
    } else {
        // Magenta for high bytes (>= 0x80)
        format!("\x1b[35m{}\x1b[0m", text)
    }
}

/// Formats the summary footer with total size, viewed range, file signature, and entropy.
fn format_summary_footer(
    total_size: u64,
    offset: u64,
    view_len: usize,
    signature: Option<&FileSignatureInfo>,
    entropy: f64,
    stats: &ByteStatistics,
) -> String {
    let mut s = String::new();
    s.push_str("\n--- Summary ---\n");

    let end_offset = offset + (view_len as u64).saturating_sub(if view_len > 0 { 1 } else { 0 });
    s.push_str(&format!(
        "Range: 0x{:X} - 0x{:X} ({} bytes viewed / {} total bytes [{:.2} KB])\n",
        offset,
        end_offset,
        view_len,
        total_size,
        (total_size as f64) / 1024.0
    ));

    if let Some(sig) = signature {
        s.push_str(&format!(
            "Format: {} [{}] ({})\n",
            sig.name, sig.category, sig.description
        ));
    } else {
        s.push_str("Format: Unknown / Raw Binary\n");
    }

    let entropy_desc = if entropy < 1.0 {
        "Very Low (sparse/repetitive/zero-filled)"
    } else if entropy < 5.0 {
        "Low (plain text, structured data, source code)"
    } else if entropy < 7.0 {
        "Moderate (compiled executable, binary structure)"
    } else {
        "High (compressed or encrypted data)"
    };

    s.push_str(&format!(
        "Entropy: {:.4} bits/byte - {}\n",
        entropy, entropy_desc
    ));

    s.push_str(&format!(
        "Distribution: Null: {:.1}% | ASCII: {:.1}% | Whitespace: {:.1}% | High Bytes: {:.1}%\n",
        stats.null_percentage,
        stats.printable_percentage,
        stats.whitespace_percentage,
        stats.high_percentage
    ));

    s
}

// ===========================================================================
// Shannon Entropy & Byte Statistics
// ===========================================================================

/// Calculates the Shannon Entropy of a byte slice in bits per byte (0.00 to 8.00).
pub fn shannon_entropy(data: &[u8]) -> f64 {
    if data.is_empty() {
        return 0.0;
    }

    let mut counts = [0usize; 256];
    for &b in data {
        counts[b as usize] += 1;
    }

    let len = data.len() as f64;
    let mut entropy = 0.0;

    for &count in &counts {
        if count > 0 {
            let p = count as f64 / len;
            entropy -= p * p.log2();
        }
    }

    entropy
}

/// Analyzes byte frequencies, categories, and distribution.
pub fn analyze_byte_distribution(data: &[u8]) -> ByteStatistics {
    let total_bytes = data.len();
    if total_bytes == 0 {
        return ByteStatistics {
            total_bytes: 0,
            null_bytes: 0,
            null_percentage: 0.0,
            printable_bytes: 0,
            printable_percentage: 0.0,
            whitespace_bytes: 0,
            whitespace_percentage: 0.0,
            control_bytes: 0,
            control_percentage: 0.0,
            high_bytes: 0,
            high_percentage: 0.0,
            entropy: 0.0,
            most_frequent: Vec::new(),
        };
    }

    let mut counts = [0usize; 256];
    let mut null_bytes = 0;
    let mut printable_bytes = 0;
    let mut whitespace_bytes = 0;
    let mut control_bytes = 0;
    let mut high_bytes = 0;

    for &b in data {
        counts[b as usize] += 1;

        if b == 0 {
            null_bytes += 1;
        }
        if (0x20..=0x7E).contains(&b) {
            printable_bytes += 1;
        }
        if b == 0x09 || b == 0x0A || b == 0x0D || b == 0x20 {
            whitespace_bytes += 1;
        }
        if b < 0x20 && b != 0x09 && b != 0x0A && b != 0x0D {
            control_bytes += 1;
        }
        if b >= 0x80 {
            high_bytes += 1;
        }
    }

    let total = total_bytes as f64;
    let mut freq_vec: Vec<(u8, usize)> = counts
        .iter()
        .enumerate()
        .filter(|(_, &c)| c > 0)
        .map(|(b, &c)| (b as u8, c))
        .collect();

    freq_vec.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    let most_frequent = freq_vec
        .into_iter()
        .take(8)
        .map(|(byte, count)| ByteFrequency {
            byte,
            hex: format!("0x{:02X}", byte),
            count,
            percentage: ((count as f64 / total) * 10000.0).round() as u32,
        })
        .collect();

    ByteStatistics {
        total_bytes,
        null_bytes,
        null_percentage: (null_bytes as f64 / total) * 100.0,
        printable_bytes,
        printable_percentage: (printable_bytes as f64 / total) * 100.0,
        whitespace_bytes,
        whitespace_percentage: (whitespace_bytes as f64 / total) * 100.0,
        control_bytes,
        control_percentage: (control_bytes as f64 / total) * 100.0,
        high_bytes,
        high_percentage: (high_bytes as f64 / total) * 100.0,
        entropy: shannon_entropy(data),
        most_frequent,
    }
}

// ===========================================================================
// File Signature Detection (Magic Numbers)
// ===========================================================================

/// Detects file format from binary magic numbers across 50+ common formats.
pub fn detect_file_type(data: &[u8]) -> Option<FileSignatureInfo> {
    if data.is_empty() {
        return None;
    }

    // 1. Executables & Binaries
    if data.len() >= 4 && &data[0..4] == b"\x7FELF" {
        let bitness = if data.len() > 4 && data[4] == 2 { "64-bit" } else { "32-bit" };
        let endian = if data.len() > 5 && data[5] == 1 { "LSB (little endian)" } else { "MSB (big endian)" };
        return Some(FileSignatureInfo {
            name: "ELF".to_string(),
            category: "Executable".to_string(),
            mime_type: Some("application/x-executable".to_string()),
            description: format!("ELF {} {} Executable/Object", bitness, endian),
            extension: "elf".to_string(),
        });
    }

    // Mach-O Binaries
    if data.len() >= 4 {
        match &data[0..4] {
            b"\xFE\xED\xFA\xCE" => return Some(FileSignatureInfo {
                name: "Mach-O 32-bit".to_string(),
                category: "Executable".to_string(),
                mime_type: Some("application/x-mach-binary".to_string()),
                description: "Mach-O 32-bit Binary (Big Endian)".to_string(),
                extension: "macho".to_string(),
            }),
            b"\xCE\xFA\xED\xFE" => return Some(FileSignatureInfo {
                name: "Mach-O 32-bit".to_string(),
                category: "Executable".to_string(),
                mime_type: Some("application/x-mach-binary".to_string()),
                description: "Mach-O 32-bit Binary (Little Endian)".to_string(),
                extension: "macho".to_string(),
            }),
            b"\xFE\xED\xFA\xCF" => return Some(FileSignatureInfo {
                name: "Mach-O 64-bit".to_string(),
                category: "Executable".to_string(),
                mime_type: Some("application/x-mach-binary".to_string()),
                description: "Mach-O 64-bit Binary (Big Endian)".to_string(),
                extension: "dylib".to_string(),
            }),
            b"\xCF\xFA\xED\xFE" => return Some(FileSignatureInfo {
                name: "Mach-O 64-bit".to_string(),
                category: "Executable".to_string(),
                mime_type: Some("application/x-mach-binary".to_string()),
                description: "Mach-O 64-bit Binary (Little Endian)".to_string(),
                extension: "dylib".to_string(),
            }),
            b"\xCA\xFE\xBA\xBE" => {
                // Disambiguate Java class vs Mach-O Universal/Fat binary
                if data.len() >= 8 && data[4] == 0 && data[5] <= 65 {
                    return Some(FileSignatureInfo {
                        name: "Java Class".to_string(),
                        category: "Bytecode".to_string(),
                        mime_type: Some("application/java-vm".to_string()),
                        description: format!("Compiled Java Class (Major: {})", data[7]),
                        extension: "class".to_string(),
                    });
                }
                return Some(FileSignatureInfo {
                    name: "Mach-O Universal Binary".to_string(),
                    category: "Executable".to_string(),
                    mime_type: Some("application/x-mach-binary".to_string()),
                    description: "Mach-O Fat / Multi-Architecture Binary".to_string(),
                    extension: "macho".to_string(),
                });
            }
            b"\x00asm" => return Some(FileSignatureInfo {
                name: "WebAssembly".to_string(),
                category: "Bytecode".to_string(),
                mime_type: Some("application/wasm".to_string()),
                description: "WebAssembly Binary Module (.wasm)".to_string(),
                extension: "wasm".to_string(),
            }),
            _ => {}
        }
    }

    // Windows PE / DOS Executables
    if data.len() >= 2 && &data[0..2] == b"MZ" {
        let is_pe = if data.len() >= 0x40 {
            let pe_offset = u32::from_le_bytes([
                data[0x3C],
                data[0x3C + 1],
                data[0x3C + 2],
                data[0x3C + 3],
            ]) as usize;
            if data.len() >= pe_offset + 4 && &data[pe_offset..pe_offset + 4] == b"PE\x00\x00" {
                true
            } else {
                false
            }
        } else {
            false
        };

        if is_pe {
            return Some(FileSignatureInfo {
                name: "Windows PE".to_string(),
                category: "Executable".to_string(),
                mime_type: Some("application/vnd.microsoft.portable-executable".to_string()),
                description: "Windows Portable Executable (EXE/DLL)".to_string(),
                extension: "exe".to_string(),
            });
        }

        return Some(FileSignatureInfo {
            name: "MS-DOS MZ".to_string(),
            category: "Executable".to_string(),
            mime_type: Some("application/x-dosexec".to_string()),
            description: "MS-DOS MZ Executable".to_string(),
            extension: "exe".to_string(),
        });
    }

    // 2. Images
    if data.len() >= 8 && &data[0..8] == b"\x89PNG\r\n\x1a\n" {
        return Some(FileSignatureInfo {
            name: "PNG Image".to_string(),
            category: "Image".to_string(),
            mime_type: Some("image/png".to_string()),
            description: "Portable Network Graphics image".to_string(),
            extension: "png".to_string(),
        });
    }

    if data.len() >= 3 && &data[0..3] == b"\xFF\xD8\xFF" {
        return Some(FileSignatureInfo {
            name: "JPEG Image".to_string(),
            category: "Image".to_string(),
            mime_type: Some("image/jpeg".to_string()),
            description: "Joint Photographic Experts Group image".to_string(),
            extension: "jpg".to_string(),
        });
    }

    if data.len() >= 6 && (&data[0..6] == b"GIF87a" || &data[0..6] == b"GIF89a") {
        return Some(FileSignatureInfo {
            name: "GIF Image".to_string(),
            category: "Image".to_string(),
            mime_type: Some("image/gif".to_string()),
            description: "Graphics Interchange Format image".to_string(),
            extension: "gif".to_string(),
        });
    }

    if data.len() >= 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        return Some(FileSignatureInfo {
            name: "WebP Image".to_string(),
            category: "Image".to_string(),
            mime_type: Some("image/webp".to_string()),
            description: "Google WebP image format".to_string(),
            extension: "webp".to_string(),
        });
    }

    if data.len() >= 2 && &data[0..2] == b"BM" {
        return Some(FileSignatureInfo {
            name: "BMP Image".to_string(),
            category: "Image".to_string(),
            mime_type: Some("image/bmp".to_string()),
            description: "Windows Bitmap image".to_string(),
            extension: "bmp".to_string(),
        });
    }

    if data.len() >= 4 && (&data[0..4] == b"II*\x00" || &data[0..4] == b"MM\x00*") {
        return Some(FileSignatureInfo {
            name: "TIFF Image".to_string(),
            category: "Image".to_string(),
            mime_type: Some("image/tiff".to_string()),
            description: "Tagged Image File Format".to_string(),
            extension: "tiff".to_string(),
        });
    }

    if data.len() >= 4 && &data[0..4] == b"\x00\x00\x01\x00" {
        return Some(FileSignatureInfo {
            name: "ICO Icon".to_string(),
            category: "Image".to_string(),
            mime_type: Some("image/x-icon".to_string()),
            description: "Windows Icon image".to_string(),
            extension: "ico".to_string(),
        });
    }

    // 3. Archives & Compressed Files
    if data.len() >= 4 && &data[0..4] == b"PK\x03\x04" {
        return Some(FileSignatureInfo {
            name: "ZIP Archive".to_string(),
            category: "Archive".to_string(),
            mime_type: Some("application/zip".to_string()),
            description: "ZIP compressed archive (or JAR/APK/DOCX/XLSX container)".to_string(),
            extension: "zip".to_string(),
        });
    }

    if data.len() >= 2 && &data[0..2] == b"\x1F\x8B" {
        return Some(FileSignatureInfo {
            name: "GZIP Archive".to_string(),
            category: "Archive".to_string(),
            mime_type: Some("application/gzip".to_string()),
            description: "GNU Gzip compressed archive".to_string(),
            extension: "gz".to_string(),
        });
    }

    if data.len() >= 3 && &data[0..3] == b"BZh" {
        return Some(FileSignatureInfo {
            name: "BZIP2 Archive".to_string(),
            category: "Archive".to_string(),
            mime_type: Some("application/x-bzip2".to_string()),
            description: "bzip2 compressed archive".to_string(),
            extension: "bz2".to_string(),
        });
    }

    if data.len() >= 6 && &data[0..6] == b"\xFD7zXZ\x00" {
        return Some(FileSignatureInfo {
            name: "XZ Archive".to_string(),
            category: "Archive".to_string(),
            mime_type: Some("application/x-xz".to_string()),
            description: "XZ compressed container".to_string(),
            extension: "xz".to_string(),
        });
    }

    if data.len() >= 6 && &data[0..6] == b"7z\xBC\xAF\x27\x1C" {
        return Some(FileSignatureInfo {
            name: "7-Zip Archive".to_string(),
            category: "Archive".to_string(),
            mime_type: Some("application/x-7z-compressed".to_string()),
            description: "7-Zip compressed archive".to_string(),
            extension: "7z".to_string(),
        });
    }

    if data.len() >= 7 && (&data[0..7] == b"Rar!\x1A\x07\x00" || &data[0..7] == b"Rar!\x1A\x07\x01") {
        return Some(FileSignatureInfo {
            name: "RAR Archive".to_string(),
            category: "Archive".to_string(),
            mime_type: Some("application/vnd.rar".to_string()),
            description: "Roshal Archive compressed file".to_string(),
            extension: "rar".to_string(),
        });
    }

    if data.len() >= 4 && &data[0..4] == b"(\xB5/\xFD" {
        return Some(FileSignatureInfo {
            name: "Zstandard".to_string(),
            category: "Archive".to_string(),
            mime_type: Some("application/zstd".to_string()),
            description: "Zstandard compressed stream".to_string(),
            extension: "zst".to_string(),
        });
    }

    if data.len() >= 262 && &data[257..262] == b"ustar" {
        return Some(FileSignatureInfo {
            name: "TAR Archive".to_string(),
            category: "Archive".to_string(),
            mime_type: Some("application/x-tar".to_string()),
            description: "POSIX Tape Archive (tar)".to_string(),
            extension: "tar".to_string(),
        });
    }

    // 4. Databases & Storage
    if data.len() >= 16 && &data[0..16] == b"SQLite format 3\x00" {
        return Some(FileSignatureInfo {
            name: "SQLite Database".to_string(),
            category: "Database".to_string(),
            mime_type: Some("application/vnd.sqlite3".to_string()),
            description: "SQLite 3 Database File".to_string(),
            extension: "sqlite".to_string(),
        });
    }

    // 5. Audio / Video / Media
    if data.len() >= 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WAVE" {
        return Some(FileSignatureInfo {
            name: "WAV Audio".to_string(),
            category: "Audio".to_string(),
            mime_type: Some("audio/wav".to_string()),
            description: "Waveform Audio File Format".to_string(),
            extension: "wav".to_string(),
        });
    }

    if data.len() >= 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"AVI " {
        return Some(FileSignatureInfo {
            name: "AVI Video".to_string(),
            category: "Video".to_string(),
            mime_type: Some("video/x-msvideo".to_string()),
            description: "Audio Video Interleave file".to_string(),
            extension: "avi".to_string(),
        });
    }

    if data.len() >= 8 && &data[4..8] == b"ftyp" {
        let brand = if data.len() >= 12 {
            std::str::from_utf8(&data[8..12]).unwrap_or("mp4")
        } else {
            "mp4"
        };
        return Some(FileSignatureInfo {
            name: "MP4 / QuickTime Video".to_string(),
            category: "Video".to_string(),
            mime_type: Some("video/mp4".to_string()),
            description: format!("ISO Base Media / MP4 / QuickTime ({})", brand.trim()),
            extension: "mp4".to_string(),
        });
    }

    if data.len() >= 3 && &data[0..3] == b"ID3" {
        return Some(FileSignatureInfo {
            name: "MP3 Audio".to_string(),
            category: "Audio".to_string(),
            mime_type: Some("audio/mpeg".to_string()),
            description: "MPEG Audio Layer III with ID3v2 tag".to_string(),
            extension: "mp3".to_string(),
        });
    }

    if data.len() >= 4 && &data[0..4] == b"fLaC" {
        return Some(FileSignatureInfo {
            name: "FLAC Audio".to_string(),
            category: "Audio".to_string(),
            mime_type: Some("audio/flac".to_string()),
            description: "Free Lossless Audio Codec".to_string(),
            extension: "flac".to_string(),
        });
    }

    if data.len() >= 4 && &data[0..4] == b"OggS" {
        return Some(FileSignatureInfo {
            name: "OGG Container".to_string(),
            category: "Audio/Video".to_string(),
            mime_type: Some("audio/ogg".to_string()),
            description: "Ogg multimedia container format".to_string(),
            extension: "ogg".to_string(),
        });
    }

    if data.len() >= 4 && &data[0..4] == b"MThd" {
        return Some(FileSignatureInfo {
            name: "MIDI Audio".to_string(),
            category: "Audio".to_string(),
            mime_type: Some("audio/midi".to_string()),
            description: "Standard MIDI Sequence".to_string(),
            extension: "mid".to_string(),
        });
    }

    // 6. Documents & Fonts
    if data.len() >= 5 && &data[0..5] == b"%PDF-" {
        let ver = if data.len() >= 8 {
            std::str::from_utf8(&data[5..8]).unwrap_or("")
        } else {
            ""
        };
        return Some(FileSignatureInfo {
            name: "PDF Document".to_string(),
            category: "Document".to_string(),
            mime_type: Some("application/pdf".to_string()),
            description: format!("Adobe Portable Document Format (v{})", ver),
            extension: "pdf".to_string(),
        });
    }

    if data.len() >= 4 && (&data[0..4] == b"\x00\x01\x00\x00" || &data[0..4] == b"true") {
        return Some(FileSignatureInfo {
            name: "TrueType Font".to_string(),
            category: "Font".to_string(),
            mime_type: Some("font/ttf".to_string()),
            description: "TrueType font file".to_string(),
            extension: "ttf".to_string(),
        });
    }

    if data.len() >= 4 && &data[0..4] == b"OTTO" {
        return Some(FileSignatureInfo {
            name: "OpenType Font".to_string(),
            category: "Font".to_string(),
            mime_type: Some("font/otf".to_string()),
            description: "OpenType font file".to_string(),
            extension: "otf".to_string(),
        });
    }

    if data.len() >= 4 && &data[0..4] == b"wOFF" {
        return Some(FileSignatureInfo {
            name: "WOFF Font".to_string(),
            category: "Font".to_string(),
            mime_type: Some("font/woff".to_string()),
            description: "Web Open Font Format".to_string(),
            extension: "woff".to_string(),
        });
    }

    if data.len() >= 4 && &data[0..4] == b"wOF2" {
        return Some(FileSignatureInfo {
            name: "WOFF2 Font".to_string(),
            category: "Font".to_string(),
            mime_type: Some("font/woff2".to_string()),
            description: "Web Open Font Format 2.0".to_string(),
            extension: "woff2".to_string(),
        });
    }

    // 7. Text BOMs & Scripts
    if data.len() >= 3 && &data[0..3] == b"\xEF\xBB\xBF" {
        return Some(FileSignatureInfo {
            name: "UTF-8 with BOM".to_string(),
            category: "Text".to_string(),
            mime_type: Some("text/plain".to_string()),
            description: "UTF-8 Encoded Text with Byte Order Mark".to_string(),
            extension: "txt".to_string(),
        });
    }

    if data.len() >= 2 && &data[0..2] == b"\xFF\xFE" {
        return Some(FileSignatureInfo {
            name: "UTF-16 LE Text".to_string(),
            category: "Text".to_string(),
            mime_type: Some("text/plain".to_string()),
            description: "UTF-16 Little Endian Encoded Text with BOM".to_string(),
            extension: "txt".to_string(),
        });
    }

    if data.len() >= 2 && &data[0..2] == b"\xFE\xFF" {
        return Some(FileSignatureInfo {
            name: "UTF-16 BE Text".to_string(),
            category: "Text".to_string(),
            mime_type: Some("text/plain".to_string()),
            description: "UTF-16 Big Endian Encoded Text with BOM".to_string(),
            extension: "txt".to_string(),
        });
    }

    if data.len() >= 2 && &data[0..2] == b"#!" {
        let first_line_end = data.iter().position(|&b| b == b'\n').unwrap_or(data.len().min(64));
        let interpreter = std::str::from_utf8(&data[2..first_line_end])
            .unwrap_or("sh")
            .trim();
        return Some(FileSignatureInfo {
            name: "Shell Script".to_string(),
            category: "Script".to_string(),
            mime_type: Some("text/x-shellscript".to_string()),
            description: format!("Executable Script (Interpreter: #{})", interpreter),
            extension: "sh".to_string(),
        });
    }

    None
}

// ===========================================================================
// String Extraction Engine (`strings` tool capability)
// ===========================================================================

/// Extracts printable ASCII and UTF-16 strings with length >= `min_length`.
pub fn extract_strings(data: &[u8], min_length: usize, offset_base: u64) -> Vec<ExtractedString> {
    let min_len = min_length.max(3);
    let mut results = Vec::new();

    // 1. ASCII / UTF-8 Extraction
    let mut current_start = None;
    let mut current_bytes = Vec::new();

    for (idx, &b) in data.iter().enumerate() {
        if (0x20..=0x7E).contains(&b) || b == 0x09 || b == 0x0A || b == 0x0D {
            if current_start.is_none() {
                current_start = Some(idx);
            }
            current_bytes.push(b);
        } else {
            if let Some(start) = current_start {
                if current_bytes.len() >= min_len {
                    if let Ok(s) = std::str::from_utf8(&current_bytes) {
                        results.push(ExtractedString {
                            offset: offset_base + start as u64,
                            length: current_bytes.len(),
                            encoding: "ASCII".to_string(),
                            value: s.trim().to_string(),
                        });
                    }
                }
            }
            current_start = None;
            current_bytes.clear();
        }
    }

    // Trailing ASCII string
    if let Some(start) = current_start {
        if current_bytes.len() >= min_len {
            if let Ok(s) = std::str::from_utf8(&current_bytes) {
                results.push(ExtractedString {
                    offset: offset_base + start as u64,
                    length: current_bytes.len(),
                    encoding: "ASCII".to_string(),
                    value: s.trim().to_string(),
                });
            }
        }
    }

    // 2. UTF-16 LE Extraction (e.g. Windows PE resources)
    if data.len() >= min_len * 2 {
        let mut u16_start = None;
        let mut u16_chars = Vec::new();

        let mut i = 0;
        while i + 1 < data.len() {
            let b0 = data[i];
            let b1 = data[i + 1];

            if (0x20..=0x7E).contains(&b0) && b1 == 0x00 {
                if u16_start.is_none() {
                    u16_start = Some(i);
                }
                u16_chars.push(b0 as char);
                i += 2;
            } else {
                if let Some(start) = u16_start {
                    if u16_chars.len() >= min_len {
                        let s: String = u16_chars.iter().collect();
                        results.push(ExtractedString {
                            offset: offset_base + start as u64,
                            length: u16_chars.len() * 2,
                            encoding: "UTF-16LE".to_string(),
                            value: s.trim().to_string(),
                        });
                    }
                }
                u16_start = None;
                u16_chars.clear();
                i += 1;
            }
        }

        if let Some(start) = u16_start {
            if u16_chars.len() >= min_len {
                let s: String = u16_chars.iter().collect();
                results.push(ExtractedString {
                    offset: offset_base + start as u64,
                    length: u16_chars.len() * 2,
                    encoding: "UTF-16LE".to_string(),
                    value: s.trim().to_string(),
                });
            }
        }
    }

    results
}

// ===========================================================================
// Pattern Searching Engine
// ===========================================================================

/// Searches for binary or text patterns in a byte slice.
/// Supports plain text, raw hex (e.g. `"7f 45 4c 46"` or `"7F454C46"`), and wildcards (`??`).
pub fn search_bytes(
    data: &[u8],
    pattern_str: &str,
    max_matches: usize,
    offset_base: u64,
) -> anyhow::Result<Vec<SearchMatch>> {
    let limit = max_matches.clamp(1, 1000);
    let mut matches = Vec::new();

    // Check if pattern is a hex string (with or without wildcard ??)
    let hex_tokens: Vec<&str> = pattern_str.split_whitespace().collect();
    let is_spaced_hex = !hex_tokens.is_empty()
        && hex_tokens.iter().all(|t| {
            t == &"??"
                || t == &"?"
                || (t.len() == 2 && t.chars().all(|c| c.is_ascii_hexdigit()))
        });

    let (pattern_bytes, pattern_mask): (Vec<u8>, Vec<bool>) = if is_spaced_hex {
        let mut bytes = Vec::new();
        let mut mask = Vec::new();
        for t in hex_tokens {
            if t == "??" || t == "?" {
                bytes.push(0);
                mask.push(false); // wildcard - ignore byte
            } else if let Ok(b) = u8::from_str_radix(t, 16) {
                bytes.push(b);
                mask.push(true);
            }
        }
        (bytes, mask)
    } else if pattern_str.starts_with("0x") || pattern_str.starts_with("0X") {
        let clean_hex = pattern_str[2..].replace(' ', "");
        let mut bytes = Vec::new();
        let mut mask = Vec::new();
        for chunk in clean_hex.as_bytes().chunks(2) {
            if let Ok(s) = std::str::from_utf8(chunk) {
                if s == "??" {
                    bytes.push(0);
                    mask.push(false);
                } else if let Ok(b) = u8::from_str_radix(s, 16) {
                    bytes.push(b);
                    mask.push(true);
                }
            }
        }
        (bytes, mask)
    } else {
        // Plain text search
        let bytes = pattern_str.as_bytes().to_vec();
        let mask = vec![true; bytes.len()];
        (bytes, mask)
    };

    if pattern_bytes.is_empty() {
        return Ok(matches);
    }

    let p_len = pattern_bytes.len();
    if data.len() < p_len {
        return Ok(matches);
    }

    for i in 0..=data.len() - p_len {
        let mut matched = true;
        for j in 0..p_len {
            if pattern_mask[j] && data[i + j] != pattern_bytes[j] {
                matched = false;
                break;
            }
        }

        if matched {
            let match_offset = offset_base + i as u64;
            let matched_slice = &data[i..i + p_len];

            // Context window: up to 8 bytes before and 8 bytes after
            let context_start = i.saturating_sub(8);
            let context_end = (i + p_len + 8).min(data.len());
            let context_slice = &data[context_start..context_end];

            let hex_preview = matched_slice
                .iter()
                .map(|b| format!("{:02X}", b))
                .collect::<Vec<_>>()
                .join(" ");

            let ascii_preview = format_ascii_representation(matched_slice);

            let context_hex = context_slice
                .iter()
                .map(|b| format!("{:02X}", b))
                .collect::<Vec<_>>()
                .join(" ");

            matches.push(SearchMatch {
                offset: match_offset,
                length: p_len,
                hex_preview,
                ascii_preview,
                context_hex,
            });

            if matches.len() >= limit {
                break;
            }
        }
    }

    Ok(matches)
}

// ===========================================================================
// Primitive Value Decoder Engine
// ===========================================================================

/// Decodes primitive types at a specific byte offset.
pub fn decode_primitives(data: &[u8], offset: usize) -> anyhow::Result<PrimitiveDecoding> {
    if offset >= data.len() {
        anyhow::bail!(
            "Decode offset {} is beyond data size ({} bytes)",
            offset,
            data.len()
        );
    }

    let slice = &data[offset..];
    let u8_val = slice[0];
    let i8_val = u8_val as i8;

    let u16_le = if slice.len() >= 2 {
        Some(u16::from_le_bytes([slice[0], slice[1]]))
    } else {
        None
    };

    let u16_be = if slice.len() >= 2 {
        Some(u16::from_be_bytes([slice[0], slice[1]]))
    } else {
        None
    };

    let i16_le = u16_le.map(|v| v as i16);
    let i16_be = u16_be.map(|v| v as i16);

    let u32_le = if slice.len() >= 4 {
        Some(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
    } else {
        None
    };

    let u32_be = if slice.len() >= 4 {
        Some(u32::from_be_bytes([slice[0], slice[1], slice[2], slice[3]]))
    } else {
        None
    };

    let i32_le = u32_le.map(|v| v as i32);
    let i32_be = u32_be.map(|v| v as i32);

    let u64_le = if slice.len() >= 8 {
        Some(u64::from_le_bytes([
            slice[0], slice[1], slice[2], slice[3], slice[4], slice[5], slice[6], slice[7],
        ]))
    } else {
        None
    };

    let u64_be = if slice.len() >= 8 {
        Some(u64::from_be_bytes([
            slice[0], slice[1], slice[2], slice[3], slice[4], slice[5], slice[6], slice[7],
        ]))
    } else {
        None
    };

    let i64_le = u64_le.map(|v| v as i64);
    let i64_be = u64_be.map(|v| v as i64);

    let f32_le = u32_le.map(f32::from_bits);
    let f32_be = u32_be.map(f32::from_bits);

    let f64_le = u64_le.map(f64::from_bits);
    let f64_be = u64_be.map(f64::from_bits);

    // C-string: read until null byte or max 128 chars
    let c_str_len = slice
        .iter()
        .take(128)
        .position(|&b| b == 0)
        .unwrap_or_else(|| slice.len().min(128));
    let c_string = if c_str_len > 0 {
        std::str::from_utf8(&slice[0..c_str_len])
            .ok()
            .map(|s| s.to_string())
    } else {
        None
    };

    let utf8_str = {
        let max_len = slice.len().min(64);
        std::str::from_utf8(&slice[0..max_len])
            .ok()
            .map(|s| s.to_string())
    };

    let unix_time_le = u32_le.and_then(|secs| {
        if (946_684_800..=2_524_608_000).contains(&secs) {
            // Reasonable range: year 2000 to 2050
            chrono::DateTime::from_timestamp(secs as i64, 0)
                .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        } else {
            None
        }
    });

    let unix_time_be = u32_be.and_then(|secs| {
        if (946_684_800..=2_524_608_000).contains(&secs) {
            chrono::DateTime::from_timestamp(secs as i64, 0)
                .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        } else {
            None
        }
    });

    Ok(PrimitiveDecoding {
        offset: offset as u64,
        u8_val,
        i8_val,
        u16_le,
        u16_be,
        i16_le,
        i16_be,
        u32_le,
        u32_be,
        i32_le,
        i32_be,
        u64_le,
        u64_be,
        i64_le,
        i64_be,
        f32_le,
        f32_be,
        f64_le,
        f64_be,
        utf8_str,
        c_string,
        unix_time_le,
        unix_time_be,
    })
}

/// Formats decoded primitive values as a clean text table.
pub fn format_primitive_decoding(d: &PrimitiveDecoding) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "=== Decoded Primitives at Offset 0x{:X} ({}) ===\n",
        d.offset, d.offset
    ));
    out.push_str(&format!(
        "{:<20} : {:<24} (hex: 0x{:02X}, bin: {:08b})\n",
        "u8 / i8",
        format!("{} / {}", d.u8_val, d.i8_val),
        d.u8_val,
        d.u8_val
    ));

    if let (Some(le), Some(be)) = (d.u16_le, d.u16_be) {
        out.push_str(&format!(
            "{:<20} : LE: {:<12} (0x{:04X}) | BE: {:<12} (0x{:04X})\n",
            "u16", le, le, be, be
        ));
    }
    if let (Some(le), Some(be)) = (d.i16_le, d.i16_be) {
        out.push_str(&format!(
            "{:<20} : LE: {:<12} (0x{:04X}) | BE: {:<12} (0x{:04X})\n",
            "i16", le, le, be, be
        ));
    }
    if let (Some(le), Some(be)) = (d.u32_le, d.u32_be) {
        out.push_str(&format!(
            "{:<20} : LE: {:<12} (0x{:08X}) | BE: {:<12} (0x{:08X})\n",
            "u32", le, le, be, be
        ));
    }
    if let (Some(le), Some(be)) = (d.i32_le, d.i32_be) {
        out.push_str(&format!(
            "{:<20} : LE: {:<12} (0x{:08X}) | BE: {:<12} (0x{:08X})\n",
            "i32", le, le, be, be
        ));
    }
    if let (Some(le), Some(be)) = (d.u64_le, d.u64_be) {
        out.push_str(&format!(
            "{:<20} : LE: {:<12} (0x{:016X}) | BE: {:<12} (0x{:016X})\n",
            "u64", le, le, be, be
        ));
    }
    if let (Some(le), Some(be)) = (d.i64_le, d.i64_be) {
        out.push_str(&format!(
            "{:<20} : LE: {:<12} (0x{:016X}) | BE: {:<12} (0x{:016X})\n",
            "i64", le, le, be, be
        ));
    }
    if let (Some(le), Some(be)) = (d.f32_le, d.f32_be) {
        out.push_str(&format!(
            "{:<20} : LE: {:<16.6} | BE: {:<16.6}\n",
            "f32 (float)", le, be
        ));
    }
    if let (Some(le), Some(be)) = (d.f64_le, d.f64_be) {
        out.push_str(&format!(
            "{:<20} : LE: {:<16.8} | BE: {:<16.8}\n",
            "f64 (double)", le, be
        ));
    }
    if let Some(s) = &d.c_string {
        out.push_str(&format!("{:<20} : {:?}\n", "C-String (null-term)", s));
    }
    if let Some(s) = &d.utf8_str {
        out.push_str(&format!("{:<20} : {:?}\n", "UTF-8 String", s));
    }
    if let Some(t) = &d.unix_time_le {
        out.push_str(&format!("{:<20} : {} (LE)\n", "Timestamp", t));
    }
    if let Some(t) = &d.unix_time_be {
        out.push_str(&format!("{:<20} : {} (BE)\n", "Timestamp", t));
    }

    out
}

// ===========================================================================
// Binary Diff Engine
// ===========================================================================

/// Compares two binary buffers and highlights differences.
pub fn binary_diff(data1: &[u8], data2: &[u8], max_chunks: usize) -> BinaryDiff {
    let limit = max_chunks.clamp(1, 500);
    let max_len = data1.len().max(data2.len());
    let mut differing_bytes_count = 0;
    let mut chunks = Vec::new();

    let mut current_chunk_start: Option<usize> = None;
    let mut file1_chunk = Vec::new();
    let mut file2_chunk = Vec::new();

    for i in 0..max_len {
        let b1 = data1.get(i).copied();
        let b2 = data2.get(i).copied();

        if b1 != b2 {
            differing_bytes_count += 1;
            if current_chunk_start.is_none() {
                current_chunk_start = Some(i);
            }
            if let Some(b) = b1 {
                file1_chunk.push(b);
            }
            if let Some(b) = b2 {
                file2_chunk.push(b);
            }
        } else {
            if let Some(start) = current_chunk_start {
                let chunk_len = file1_chunk.len().max(file2_chunk.len());
                let file1_hex = file1_chunk
                    .iter()
                    .map(|b| format!("{:02X}", b))
                    .collect::<Vec<_>>()
                    .join(" ");
                let file2_hex = file2_chunk
                    .iter()
                    .map(|b| format!("{:02X}", b))
                    .collect::<Vec<_>>()
                    .join(" ");

                chunks.push(DiffChunk {
                    offset: start as u64,
                    length: chunk_len,
                    file1_bytes: file1_chunk.clone(),
                    file2_bytes: file2_chunk.clone(),
                    file1_hex,
                    file2_hex,
                    file1_ascii: format_ascii_representation(&file1_chunk),
                    file2_ascii: format_ascii_representation(&file2_chunk),
                });

                if chunks.len() >= limit {
                    break;
                }
            }
            current_chunk_start = None;
            file1_chunk.clear();
            file2_chunk.clear();
        }
    }

    if let Some(start) = current_chunk_start {
        if chunks.len() < limit {
            let chunk_len = file1_chunk.len().max(file2_chunk.len());
            let file1_hex = file1_chunk
                .iter()
                .map(|b| format!("{:02X}", b))
                .collect::<Vec<_>>()
                .join(" ");
            let file2_hex = file2_chunk
                .iter()
                .map(|b| format!("{:02X}", b))
                .collect::<Vec<_>>()
                .join(" ");

            chunks.push(DiffChunk {
                offset: start as u64,
                length: chunk_len,
                file1_bytes: file1_chunk,
                file2_bytes: file2_chunk,
                file1_hex,
                file2_hex,
                file1_ascii: format_ascii_representation(&data1[start..data1.len().min(start + chunk_len)]),
                file2_ascii: format_ascii_representation(&data2[start..data2.len().min(start + chunk_len)]),
            });
        }
    }

    let diff_pct = if max_len > 0 {
        (differing_bytes_count as f64 / max_len as f64) * 100.0
    } else {
        0.0
    };

    BinaryDiff {
        file1_size: data1.len() as u64,
        file2_size: data2.len() as u64,
        differing_bytes_count,
        differing_percentage: diff_pct,
        chunks,
    }
}

/// Formats a binary diff as human-readable text.
pub fn format_binary_diff(diff: &BinaryDiff) -> String {
    let mut out = String::new();
    out.push_str("=== Binary Comparison Diff ===\n");
    out.push_str(&format!(
        "File 1: {} bytes | File 2: {} bytes\n",
        diff.file1_size, diff.file2_size
    ));
    out.push_str(&format!(
        "Differences: {} bytes ({:.2}% difference)\n\n",
        diff.differing_bytes_count, diff.differing_percentage
    ));

    if diff.differing_bytes_count == 0 {
        out.push_str("Files are identical.\n");
        return out;
    }

    out.push_str(&format!("{:<10} {:<24} {:<24} {}\n", "Offset", "File 1 Hex", "File 2 Hex", "Decoded"));
    out.push_str(&format!("{}\n", "-".repeat(70)));

    for chunk in &diff.chunks {
        out.push_str(&format!(
            "0x{:08X}   {:<24} {:<24} |{}| vs |{}|\n",
            chunk.offset,
            chunk.file1_hex,
            chunk.file2_hex,
            chunk.file1_ascii,
            chunk.file2_ascii
        ));
    }

    if diff.chunks.len() >= 500 {
        out.push_str("... (diff truncated at 500 chunks)\n");
    }

    out
}

// ===========================================================================
// Input Parsing Helpers
// ===========================================================================

/// Parses an offset or length from either a JSON integer, float, or string (decimal, hex `"0x100"`, or unit `"1k"`).
pub fn parse_size_or_offset(val: &Value) -> Option<u64> {
    if let Some(n) = val.as_u64() {
        return Some(n);
    }
    if let Some(i) = val.as_i64() {
        return Some(i.max(0) as u64);
    }
    if let Some(s) = val.as_str() {
        let trimmed = s.trim();
        if trimmed.starts_with("0x") || trimmed.starts_with("0X") {
            return u64::from_str_radix(&trimmed[2..], 16).ok();
        }
        if trimmed.starts_with('$') {
            return u64::from_str_radix(&trimmed[1..], 16).ok();
        }

        // Check for suffixes k, m, g
        let lower = trimmed.to_lowercase();
        if let Some(num_str) = lower.strip_suffix("kb").or_else(|| lower.strip_suffix('k')) {
            return num_str.trim().parse::<f64>().ok().map(|n| (n * 1024.0) as u64);
        }
        if let Some(num_str) = lower.strip_suffix("mb").or_else(|| lower.strip_suffix('m')) {
            return num_str
                .trim()
                .parse::<f64>()
                .ok()
                .map(|n| (n * 1024.0 * 1024.0) as u64);
        }
        if let Some(num_str) = lower.strip_suffix("gb").or_else(|| lower.strip_suffix('g')) {
            return num_str
                .trim()
                .parse::<f64>()
                .ok()
                .map(|n| (n * 1024.0 * 1024.0 * 1024.0) as u64);
        }

        return trimmed.parse::<u64>().ok();
    }
    None
}

/// Parses raw byte data from either direct hex string, base64 string, or raw text.
pub fn parse_raw_data_input(raw: &str) -> Option<Vec<u8>> {
    let trimmed = raw.trim();

    // 1. Spaced hex or 0x-prefixed hex: "7f 45 4c 46" or "0x7F454C46"
    if trimmed.starts_with("0x") || trimmed.starts_with("0X") {
        let hex_str = trimmed[2..].replace([' ', '\n', '\r', '\t'], "");
        if hex_str.len() % 2 == 0 && hex_str.chars().all(|c| c.is_ascii_hexdigit()) {
            let mut bytes = Vec::new();
            for chunk in hex_str.as_bytes().chunks(2) {
                if let Ok(s) = std::str::from_utf8(chunk) {
                    if let Ok(b) = u8::from_str_radix(s, 16) {
                        bytes.push(b);
                    }
                }
            }
            return Some(bytes);
        }
    }

    // Spaced hex tokens: "7f 45 4c 46 02"
    let tokens: Vec<&str> = trimmed.split_whitespace().collect();
    if tokens.len() >= 2
        && tokens
            .iter()
            .all(|t| t.len() == 2 && t.chars().all(|c| c.is_ascii_hexdigit()))
    {
        let mut bytes = Vec::new();
        for t in tokens {
            if let Ok(b) = u8::from_str_radix(t, 16) {
                bytes.push(b);
            }
        }
        return Some(bytes);
    }

    // 2. Base64 decoded check (pure Rust simple base64 decoder)
    if let Some(b64_bytes) = decode_base64_simple(trimmed) {
        if !b64_bytes.is_empty() {
            return Some(b64_bytes);
        }
    }

    // 3. Fallback: raw UTF-8 bytes
    Some(raw.as_bytes().to_vec())
}

/// Simple pure-Rust Base64 decoder without extra external dependencies.
fn decode_base64_simple(s: &str) -> Option<Vec<u8>> {
    let clean: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if clean.is_empty() || clean.len() % 4 != 0 {
        return None;
    }

    let b64_table = |c: u8| -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' | b'-' => Some(62),
            b'/' | b'_' => Some(63),
            b'=' => Some(0),
            _ => None,
        }
    };

    let bytes = clean.as_bytes();
    let mut out = Vec::with_capacity((clean.len() / 4) * 3);

    for chunk in bytes.chunks(4) {
        let a = b64_table(chunk[0])?;
        let b = b64_table(chunk[1])?;
        let c = b64_table(chunk[2])?;
        let d = b64_table(chunk[3])?;

        out.push((a << 2) | (b >> 4));
        if chunk[2] != b'=' {
            out.push(((b & 0xF) << 4) | (c >> 2));
        }
        if chunk[3] != b'=' {
            out.push(((c & 0x3) << 6) | d);
        }
    }

    Some(out)
}

// ===========================================================================
// HexViewerTool (Tool implementation)
// ===========================================================================

/// The Hexadecimal and Binary File Viewer Tool.
#[derive(Default, Debug, Clone)]
pub struct HexViewerTool;

impl HexViewerTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for HexViewerTool {
    fn name(&self) -> &str {
        "hex_view"
    }

    fn description(&self) -> &str {
        "Hexadecimal and binary file viewer tool with configurable byte grouping, radix formatting, ASCII inspection, pattern searching, entropy analysis, file signature detection, primitive struct decoding, and binary diffing."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the binary or text file to inspect."
                },
                "data": {
                    "type": "string",
                    "description": "Direct binary input as raw text, hex string (e.g. '7F 45 4C 46'), or base64 (used if path is not specified)."
                },
                "action": {
                    "type": "string",
                    "enum": ["dump", "inspect", "search", "strings", "decode", "diff"],
                    "description": "Action mode to execute: 'dump' (canonical hex dump), 'inspect' (metadata/entropy/stats), 'search' (binary pattern finder), 'strings' (extract ASCII/UTF-16 text), 'decode' (decode primitives at offset), or 'diff' (binary file comparison). Default: 'dump'."
                },
                "offset": {
                    "type": ["integer", "string"],
                    "description": "0-based byte offset to start reading from (supports integer, hex '0x100', or units '1k', '1m'). Default: 0."
                },
                "length": {
                    "type": ["integer", "string"],
                    "description": "Maximum number of bytes to read/dump (default: 256 bytes for files; None for unlimited)."
                },
                "bytes_per_row": {
                    "type": "integer",
                    "description": "Number of bytes displayed per row (e.g. 8, 16, 24, 32, 64). Default: 16."
                },
                "group_size": {
                    "type": "integer",
                    "description": "Number of bytes per space-separated hex group (e.g. 1, 2, 4, 8). Default: 1."
                },
                "radix": {
                    "type": "string",
                    "enum": ["hex", "dec", "oct"],
                    "description": "Offset display radix ('hex', 'dec', or 'oct'). Default: 'hex'."
                },
                "byte_format": {
                    "type": "string",
                    "enum": ["hex", "HEX", "bin", "dec", "oct"],
                    "description": "Byte representation format ('hex', 'HEX', 'bin', 'dec', or 'oct'). Default: 'hex'."
                },
                "show_ascii": {
                    "type": "boolean",
                    "description": "Whether to display the decoded ASCII column. Default: true."
                },
                "show_header": {
                    "type": "boolean",
                    "description": "Whether to display the column header. Default: true."
                },
                "show_summary": {
                    "type": "boolean",
                    "description": "Whether to display the summary footer with range, file format, and entropy. Default: true."
                },
                "collapse_repeats": {
                    "type": "boolean",
                    "description": "Whether to squeeze consecutive identical rows with '*'. Default: false."
                },
                "search": {
                    "type": "string",
                    "description": "Byte pattern or string to search for (supports hex '7f 45 4c 46', wildcards '7f ?? 4c', or plain text)."
                },
                "min_string_length": {
                    "type": "integer",
                    "description": "Minimum string length for 'strings' extraction mode. Default: 4."
                },
                "decode_offset": {
                    "type": ["integer", "string"],
                    "description": "Byte offset to decode primitive types (u8, u16, u32, u64, float, double, string, timestamp)."
                },
                "diff_path": {
                    "type": "string",
                    "description": "Second file path for binary diff comparison mode ('diff')."
                },
                "format": {
                    "type": "string",
                    "enum": ["text", "json"],
                    "description": "Output format: 'text' (human-readable) or 'json' (structured data). Default: 'text'."
                }
            }
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> anyhow::Result<String> {
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .map(HexAction::from_str_loose)
            .unwrap_or(HexAction::Dump);

        let output_format = args
            .get("format")
            .and_then(|v| v.as_str())
            .map(OutputFormat::from_str_loose)
            .unwrap_or(OutputFormat::Text);

        // 1. Acquire primary byte data (either from file path or direct data)
        let path_str = args
            .get("path")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("file_path").and_then(|v| v.as_str()));

        let data: Vec<u8> = if let Some(p) = path_str {
            let full_path = resolve_path(p, &ctx.cwd);
            if !full_path.exists() {
                anyhow::bail!("File not found: '{}'", full_path.display());
            }
            if full_path.is_dir() {
                anyhow::bail!("Path is a directory, not a file: '{}'", full_path.display());
            }

            tokio::fs::read(&full_path)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to read file '{}': {e}", full_path.display()))?
        } else if let Some(raw_data) = args
            .get("data")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("bytes").and_then(|v| v.as_str()))
        {
            parse_raw_data_input(raw_data).unwrap_or_else(|| raw_data.as_bytes().to_vec())
        } else {
            anyhow::bail!("Missing required parameter: either 'path' or 'data' must be provided");
        };

        // 2. Options Extraction
        let offset = args
            .get("offset")
            .or_else(|| args.get("seek"))
            .or_else(|| args.get("start"))
            .and_then(parse_size_or_offset)
            .unwrap_or(0);

        let length = args
            .get("length")
            .or_else(|| args.get("limit"))
            .or_else(|| args.get("count"))
            .and_then(parse_size_or_offset)
            .map(|l| l as usize)
            .or(Some(256));

        let bytes_per_row = args
            .get("bytes_per_row")
            .or_else(|| args.get("columns"))
            .or_else(|| args.get("width"))
            .and_then(|v| v.as_u64())
            .unwrap_or(16) as usize;

        let group_size = args
            .get("group_size")
            .or_else(|| args.get("grouping"))
            .and_then(|v| v.as_u64())
            .unwrap_or(1) as usize;

        let offset_radix = args
            .get("radix")
            .or_else(|| args.get("offset_format"))
            .and_then(|v| v.as_str())
            .map(OffsetRadix::from_str_loose)
            .unwrap_or(OffsetRadix::Hex);

        let byte_format = args
            .get("byte_format")
            .and_then(|v| v.as_str())
            .map(ByteFormat::from_str_loose)
            .unwrap_or(ByteFormat::HexLower);

        let show_ascii = args
            .get("show_ascii")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let show_header = args
            .get("show_header")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let show_summary = args
            .get("show_summary")
            .or_else(|| args.get("show_stats"))
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let collapse_repeats = args
            .get("collapse_repeats")
            .or_else(|| args.get("squeeze"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let options = HexDumpOptions {
            offset,
            length,
            bytes_per_row,
            group_size,
            offset_radix,
            byte_format,
            show_ascii,
            show_header,
            show_summary,
            collapse_repeats,
            color: false,
        };

        // 3. Dispatch Action
        match action {
            HexAction::Dump => {
                let dump_res = hex_dump(&data, &options);
                match output_format {
                    OutputFormat::Text => Ok(dump_res.formatted_output),
                    OutputFormat::Json => Ok(serde_json::to_string_pretty(&dump_res)?),
                }
            }
            HexAction::Inspect => {
                let sig = detect_file_type(&data);
                let entropy = shannon_entropy(&data);
                let stats = analyze_byte_distribution(&data);

                if output_format == OutputFormat::Json {
                    return Ok(serde_json::to_string_pretty(&json!({
                        "total_size": data.len(),
                        "file_type": sig,
                        "entropy": entropy,
                        "byte_statistics": stats,
                    }))?);
                }

                let mut out = String::new();
                out.push_str("=== Binary File Inspection & Analysis ===\n");
                out.push_str(&format!("Total Size: {} bytes ({:.2} KB)\n", data.len(), (data.len() as f64) / 1024.0));

                if let Some(s) = sig {
                    out.push_str(&format!("File Type: {} ({})\n", s.name, s.category));
                    out.push_str(&format!("Description: {}\n", s.description));
                    if let Some(mime) = s.mime_type {
                        out.push_str(&format!("MIME Type: {}\n", mime));
                    }
                } else {
                    out.push_str("File Type: Unknown / Generic Binary\n");
                }

                out.push_str(&format!("Shannon Entropy: {:.4} bits/byte\n", entropy));
                out.push_str(&format!(
                    "Null Bytes: {} ({:.2}%)\n",
                    stats.null_bytes, stats.null_percentage
                ));
                out.push_str(&format!(
                    "Printable ASCII: {} ({:.2}%)\n",
                    stats.printable_bytes, stats.printable_percentage
                ));
                out.push_str(&format!(
                    "Whitespace: {} ({:.2}%)\n",
                    stats.whitespace_bytes, stats.whitespace_percentage
                ));
                out.push_str(&format!(
                    "Control Chars: {} ({:.2}%)\n",
                    stats.control_bytes, stats.control_percentage
                ));
                out.push_str(&format!(
                    "High Bytes: {} ({:.2}%)\n",
                    stats.high_bytes, stats.high_percentage
                ));

                out.push_str("\nMost Frequent Bytes:\n");
                for freq in &stats.most_frequent {
                    out.push_str(&format!(
                        "  {} (byte: {:03}) : {:<8} ({:.2}%)\n",
                        freq.hex,
                        freq.byte,
                        freq.count,
                        freq.percentage as f64 / 100.0
                    ));
                }

                Ok(out)
            }
            HexAction::Search => {
                let pattern = args
                    .get("search")
                    .or_else(|| args.get("pattern"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing required parameter: 'search' pattern"))?;

                let matches = search_bytes(&data, pattern, 100, 0)?;

                if output_format == OutputFormat::Json {
                    return Ok(serde_json::to_string_pretty(&json!({
                        "pattern": pattern,
                        "matches_count": matches.len(),
                        "matches": matches,
                    }))?);
                }

                let mut out = String::new();
                out.push_str(&format!("=== Pattern Search: {:?} ===\n", pattern));
                out.push_str(&format!("Matches Found: {}\n\n", matches.len()));

                for (idx, m) in matches.iter().enumerate() {
                    out.push_str(&format!(
                        "Match #{}: Offset 0x{:08X} (decimal: {}) | Length: {} bytes\n",
                        idx + 1,
                        m.offset,
                        m.offset,
                        m.length
                    ));
                    out.push_str(&format!("  Hex:   {}\n", m.hex_preview));
                    out.push_str(&format!("  ASCII: |{}|\n", m.ascii_preview));
                    out.push_str(&format!("  Context: {}\n\n", m.context_hex));
                }

                if matches.is_empty() {
                    out.push_str("No occurrences found in binary data.\n");
                }

                Ok(out)
            }
            HexAction::Strings => {
                let min_len = args
                    .get("min_string_length")
                    .or_else(|| args.get("min_len"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(4) as usize;

                let strings = extract_strings(&data, min_len, 0);

                if output_format == OutputFormat::Json {
                    return Ok(serde_json::to_string_pretty(&json!({
                        "min_length": min_len,
                        "count": strings.len(),
                        "strings": strings,
                    }))?);
                }

                let mut out = String::new();
                out.push_str(&format!("=== Extracted Strings (min_length: {}) ===\n", min_len));
                out.push_str(&format!("Total Strings Found: {}\n\n", strings.len()));

                for s in strings.iter().take(200) {
                    out.push_str(&format!("0x{:08X} [{:<8}] {}\n", s.offset, s.encoding, s.value));
                }

                if strings.len() > 200 {
                    out.push_str(&format!("... (truncated {} additional strings)\n", strings.len() - 200));
                }

                Ok(out)
            }
            HexAction::Decode => {
                let decode_off = args
                    .get("decode_offset")
                    .or_else(|| args.get("offset"))
                    .and_then(parse_size_or_offset)
                    .unwrap_or(0) as usize;

                let decoded = decode_primitives(&data, decode_off)?;

                if output_format == OutputFormat::Json {
                    return Ok(serde_json::to_string_pretty(&decoded)?);
                }

                Ok(format_primitive_decoding(&decoded))
            }
            HexAction::Diff => {
                let diff_path_str = args
                    .get("diff_path")
                    .or_else(|| args.get("second_path"))
                    .or_else(|| args.get("file2"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing required parameter: 'diff_path' for binary comparison"))?;

                let full_diff_path = resolve_path(diff_path_str, &ctx.cwd);
                if !full_diff_path.exists() {
                    anyhow::bail!("Diff file not found: '{}'", full_diff_path.display());
                }

                let data2 = tokio::fs::read(&full_diff_path)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to read diff file '{}': {e}", full_diff_path.display()))?;

                let diff_res = binary_diff(&data, &data2, 500);

                if output_format == OutputFormat::Json {
                    return Ok(serde_json::to_string_pretty(&diff_res)?);
                }

                Ok(format_binary_diff(&diff_res))
            }
        }
    }
}

// ===========================================================================
// Unit Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_hex_dump_basic() {
        let sample = b"Hello, World! 12345";
        let mut opts = HexDumpOptions::default();
        opts.show_header = true;
        opts.show_summary = false;
        opts.bytes_per_row = 16;
        opts.group_size = 1;

        let output = format_hex_dump(sample, &opts);
        assert!(output.contains("Offset(h)"));
        assert!(output.contains("48 65 6c 6c 6f 2c 20 57"));
        assert!(output.contains("|Hello, World! 12|"));
    }

    #[test]
    fn test_format_hex_dump_custom_grouping_and_columns() {
        let sample = vec![0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x11, 0x22];
        let mut opts = HexDumpOptions::default();
        opts.show_header = false;
        opts.show_summary = false;
        opts.bytes_per_row = 8;
        opts.group_size = 2;
        opts.byte_format = ByteFormat::HexUpper;

        let output = format_hex_dump(&sample, &opts);
        assert!(output.contains("AABB CCDD EEFF 1122"));
    }

    #[test]
    fn test_offset_radix() {
        let sample = vec![0x10; 32];
        let mut opts = HexDumpOptions::default();
        opts.show_header = false;
        opts.show_summary = false;
        opts.bytes_per_row = 16;
        opts.offset_radix = OffsetRadix::Dec;

        let output = format_hex_dump(&sample, &opts);
        assert!(output.contains("00000000"));
        assert!(output.contains("00000016"));
    }

    #[test]
    fn test_shannon_entropy() {
        // All identical bytes -> 0.0 entropy
        let zeroes = vec![0u8; 1000];
        let ent_zeroes = shannon_entropy(&zeroes);
        assert_eq!(ent_zeroes, 0.0);

        // Uniform distribution of all 256 bytes -> 8.0 entropy
        let mut uniform = Vec::new();
        for _ in 0..10 {
            for b in 0..=255u8 {
                uniform.push(b);
            }
        }
        let ent_uniform = shannon_entropy(&uniform);
        assert!((ent_uniform - 8.0).abs() < 0.0001);

        // Plain text -> moderate entropy (3.0 - 5.0)
        let text = b"The quick brown fox jumps over the lazy dog. Programming in pure Rust is fast and safe.";
        let ent_text = shannon_entropy(text);
        assert!(ent_text > 3.0 && ent_text < 5.0);
    }

    #[test]
    fn test_file_signature_detection() {
        // ELF
        let elf = b"\x7FELF\x02\x01\x01\x00";
        let sig = detect_file_type(elf).unwrap();
        assert_eq!(sig.name, "ELF");
        assert_eq!(sig.category, "Executable");

        // PNG
        let png = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR";
        let sig = detect_file_type(png).unwrap();
        assert_eq!(sig.name, "PNG Image");

        // SQLite
        let sqlite = b"SQLite format 3\x00";
        let sig = detect_file_type(sqlite).unwrap();
        assert_eq!(sig.name, "SQLite Database");

        // ZIP
        let zip = b"PK\x03\x04\x14\x00\x00\x00";
        let sig = detect_file_type(zip).unwrap();
        assert_eq!(sig.name, "ZIP Archive");

        // PDF
        let pdf = b"%PDF-1.7\n";
        let sig = detect_file_type(pdf).unwrap();
        assert_eq!(sig.name, "PDF Document");

        // WASM
        let wasm = b"\x00asm\x01\x00\x00\x00";
        let sig = detect_file_type(wasm).unwrap();
        assert_eq!(sig.name, "WebAssembly");
    }

    #[test]
    fn test_string_extraction() {
        let mut data = vec![0x00, 0x01, 0x02];
        data.extend_from_slice(b"MAGIC_KEY_12345");
        data.extend_from_slice(&[0x00, 0xFF, 0xFE]);
        data.extend_from_slice(b"ANOTHER_STRING");
        data.push(0x00);

        let strings = extract_strings(&data, 4, 0);
        let values: Vec<&str> = strings.iter().map(|s| s.value.as_str()).collect();
        assert!(values.contains(&"MAGIC_KEY_12345"));
        assert!(values.contains(&"ANOTHER_STRING"));
    }

    #[test]
    fn test_binary_search() {
        let data = b"Some random bytes \x7F\x45\x4C\x46\x02\x01 and more text \x7F\x45\x99\x46 and end";

        // Plain string search
        let res = search_bytes(data, "random", 10, 0).unwrap();
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].offset, 5);

        // Hex search with spaces
        let res_hex = search_bytes(data, "7f 45 4c 46", 10, 0).unwrap();
        assert_eq!(res_hex.len(), 1);
        assert_eq!(res_hex[0].offset, 18);

        // Hex search with wildcard ??
        let res_wildcard = search_bytes(data, "7f 45 ?? 46", 10, 0).unwrap();
        assert_eq!(res_wildcard.len(), 2);
    }

    #[test]
    fn test_primitive_decoding() {
        let mut data = Vec::new();
        // Offset 0: u16 LE (0x1234 = 4660)
        data.extend_from_slice(&4660u16.to_le_bytes());
        // Offset 2: u32 LE (0x11223344 = 287454020)
        data.extend_from_slice(&287454020u32.to_le_bytes());
        // Offset 6: f32 LE (3.14159)
        data.extend_from_slice(&3.14159f32.to_le_bytes());
        // Offset 10: Null terminated C-string "Fusion"
        data.extend_from_slice(b"Fusion\0");

        let dec0 = decode_primitives(&data, 0).unwrap();
        assert_eq!(dec0.u16_le, Some(4660));

        let dec2 = decode_primitives(&data, 2).unwrap();
        assert_eq!(dec2.u32_le, Some(287454020));

        let dec6 = decode_primitives(&data, 6).unwrap();
        assert!((dec6.f32_le.unwrap() - 3.14159).abs() < 0.0001);

        let dec10 = decode_primitives(&data, 10).unwrap();
        assert_eq!(dec10.c_string.as_deref(), Some("Fusion"));
    }

    #[test]
    fn test_binary_diff() {
        let data1 = b"ABCDEF123456";
        let data2 = b"ABCXYZ123456";

        let diff = binary_diff(data1, data2, 10);
        assert_eq!(diff.differing_bytes_count, 3);
        assert_eq!(diff.chunks.len(), 1);
        assert_eq!(diff.chunks[0].offset, 3);
        assert_eq!(diff.chunks[0].length, 3);
    }

    #[tokio::test]
    async fn test_hex_viewer_tool_execute_direct_data() {
        let tool = HexViewerTool::new();
        let ctx = ToolContext::default();

        let args = json!({
            "data": "7f 45 4c 46 02 01 01 00",
            "action": "dump",
            "format": "text",
            "show_header": true,
            "show_summary": true
        });

        let result = tool.execute(args, &ctx).await.unwrap();
        assert!(result.contains("Offset(h)"));
        assert!(result.contains("7f 45 4c 46 02 01 01 00"));
        assert!(result.contains("|.ELF....|"));
        assert!(result.contains("ELF"));
    }

    #[tokio::test]
    async fn test_hex_viewer_tool_json_output() {
        let tool = HexViewerTool::new();
        let ctx = ToolContext::default();

        let args = json!({
            "data": "SGVsbG8gV29ybGQ=", // Base64 for "Hello World"
            "action": "dump",
            "format": "json"
        });

        let result = tool.execute(args, &ctx).await.unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["total_size"], 11);
        assert!(parsed["rows"].as_array().unwrap().len() > 0);
    }

    #[tokio::test]
    async fn test_hex_viewer_tool_file_inspect() {
        let tool = HexViewerTool::new();
        let ctx = ToolContext::default();

        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("sample.bin");
        std::fs::write(&file_path, b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR").unwrap();

        let args = json!({
            "path": file_path.to_str().unwrap(),
            "action": "inspect",
            "format": "text"
        });

        let result = tool.execute(args, &ctx).await.unwrap();
        assert!(result.contains("PNG Image"));
        assert!(result.contains("Shannon Entropy"));
    }
}
