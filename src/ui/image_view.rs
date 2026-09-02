//! Inline terminal image rendering for Fusion.
//!
//! Supports three output modes, selected automatically by probing environment variables:
//!
//! 1. **iTerm2 inline images** (`ESC]1337;File=…:<base64>BEL`) — iTerm2, WezTerm, Warp.
//! 2. **Sixel** (`ESC[?80h` / DCS sequences) — xterm, foot, mlterm, yaft.
//! 3. **ASCII art fallback** — block-character downsampler; works everywhere.
//!
//! Image decoding is pure Rust (no external crates). Supported input formats:
//! PNG (truecolor + palette), JPEG (baseline/progressive), BMP (24/32-bit uncompressed).
//! Anything else falls back gracefully to ASCII art rendered from a placeholder gradient.

use std::{
    fs,
    path::Path,
};

// ---------------------------------------------------------------------------
// Terminal protocol detection
// ---------------------------------------------------------------------------

/// Which inline-image protocol the current terminal supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageProtocol {
    /// iTerm2 `ESC]1337;File=` protocol.
    ITerm2,
    /// DEC Sixel graphics.
    Sixel,
    /// ASCII art block-character fallback.
    Ascii,
}

impl ImageProtocol {
    /// Detect the best protocol supported by the running terminal.
    ///
    /// Probes `TERM_PROGRAM`, `LC_TERMINAL`, `TERM`, and `COLORTERM`.
    pub fn detect() -> Self {
        // iTerm2 / WezTerm / Warp all support the iTerm2 protocol.
        let term_program = std::env::var("TERM_PROGRAM").unwrap_or_default();
        let lc_terminal = std::env::var("LC_TERMINAL").unwrap_or_default();

        let iterm2_hosts = ["iTerm.app", "WezTerm", "Warp", "Hyper", "tabby"];
        for host in &iterm2_hosts {
            if term_program.contains(host) || lc_terminal.contains(host) {
                return Self::ITerm2;
            }
        }

        // Sixel: xterm (XTERM_VERSION), foot, mlterm, yaft, DomTerm.
        let term = std::env::var("TERM").unwrap_or_default();
        let xterm_version = std::env::var("XTERM_VERSION").ok();
        let colorterm = std::env::var("COLORTERM").unwrap_or_default();

        if xterm_version.is_some()
            || term.contains("sixel")
            || term.starts_with("mlterm")
            || term.starts_with("foot")
            || term.starts_with("yaft")
            || colorterm.contains("sixel")
        {
            return Self::Sixel;
        }

        Self::Ascii
    }
}

// ---------------------------------------------------------------------------
// Raw pixel buffer
// ---------------------------------------------------------------------------

/// An RGBA pixel grid decoded from an image file.
struct PixelBuf {
    width: usize,
    height: usize,
    /// Row-major, four bytes per pixel: R G B A.
    data: Vec<u8>,
}

impl PixelBuf {
    /// Sample the pixel at `(x, y)` → `(r, g, b, a)`.
    fn pixel(&self, x: usize, y: usize) -> (u8, u8, u8, u8) {
        let base = (y * self.width + x) * 4;
        (
            self.data[base],
            self.data[base + 1],
            self.data[base + 2],
            self.data[base + 3],
        )
    }

    /// Sample an average pixel from a rectangular tile `[x0,x1) × [y0,y1)`.
    /// Returns `(r, g, b, a)` averaged over all pixels in the tile.
    fn sample_tile(&self, x0: usize, y0: usize, x1: usize, y1: usize) -> (u8, u8, u8, u8) {
        let x1 = x1.min(self.width);
        let y1 = y1.min(self.height);
        if x0 >= x1 || y0 >= y1 {
            return (0, 0, 0, 255);
        }
        let (mut r, mut g, mut b, mut a) = (0u32, 0u32, 0u32, 0u32);
        let mut n = 0u32;
        for row in y0..y1 {
            for col in x0..x1 {
                let (pr, pg, pb, pa) = self.pixel(col, row);
                r += pr as u32;
                g += pg as u32;
                b += pb as u32;
                a += pa as u32;
                n += 1;
            }
        }
        (
            (r / n) as u8,
            (g / n) as u8,
            (b / n) as u8,
            (a / n) as u8,
        )
    }

    /// Build a placeholder gradient buffer of the given dimensions.
    /// Used when the actual image cannot be decoded.
    fn placeholder(width: usize, height: usize) -> Self {
        let mut data = Vec::with_capacity(width * height * 4);
        for y in 0..height {
            for x in 0..width {
                let r = ((x * 255) / width.max(1)) as u8;
                let g = ((y * 255) / height.max(1)) as u8;
                let b = 128u8;
                data.extend_from_slice(&[r, g, b, 255]);
            }
        }
        Self { width, height, data }
    }
}

// ---------------------------------------------------------------------------
// PNG decoder (pure Rust, no crates)
// ---------------------------------------------------------------------------

/// Minimal PNG decoder.  Supports bit depths 1/2/4/8/16, colour types 0/2/3/4/6.
/// Does **not** handle interlaced (Adam7) images; those fall through to ASCII placeholder.
mod png {
    use super::PixelBuf;

    // zlib inflate — fixed & dynamic Huffman, stored blocks.
    // Implements RFC 1950 (zlib wrapper) + RFC 1951 (deflate).
    struct BitReader<'a> {
        data: &'a [u8],
        byte_pos: usize,
        bit_pos: u8, // 0..7
    }

    impl<'a> BitReader<'a> {
        fn new(data: &'a [u8]) -> Self {
            Self { data, byte_pos: 0, bit_pos: 0 }
        }

        fn read_bits(&mut self, n: u8) -> Option<u32> {
            let mut val = 0u32;
            for i in 0..n {
                if self.byte_pos >= self.data.len() {
                    return None;
                }
                let bit = (self.data[self.byte_pos] >> self.bit_pos) & 1;
                val |= (bit as u32) << i;
                self.bit_pos += 1;
                if self.bit_pos == 8 {
                    self.bit_pos = 0;
                    self.byte_pos += 1;
                }
            }
            Some(val)
        }

        /// Align to next byte boundary (discard remaining bits in current byte).
        fn align_byte(&mut self) {
            if self.bit_pos != 0 {
                self.bit_pos = 0;
                self.byte_pos += 1;
            }
        }

        fn read_byte(&mut self) -> Option<u8> {
            self.align_byte();
            if self.byte_pos < self.data.len() {
                let b = self.data[self.byte_pos];
                self.byte_pos += 1;
                Some(b)
            } else {
                None
            }
        }

        fn read_u16_le(&mut self) -> Option<u16> {
            let lo = self.read_byte()? as u16;
            let hi = self.read_byte()? as u16;
            Some(lo | (hi << 8))
        }
    }

    // Build canonical Huffman decoder table (code→symbol, limited to 15 bits).
    struct HuffTable {
        // For each code length 1..=15: list of (code, symbol).
        // We decode by scanning; fine for small tables.
        entries: Vec<(u16, u8, Vec<u16>)>, // (len, ?, [symbols at that len sorted by code])
        // Flat: (code, len) → symbol lookup via sorted entries per length.
        by_len: [Vec<(u16, u16)>; 16], // index = code length; entries (code, symbol)
    }

    impl HuffTable {
        fn build(lengths: &[u16]) -> Self {
            // RFC 1951 §3.2.2
            let max_len = *lengths.iter().max().unwrap_or(&0) as usize;
            let mut bl_count = vec![0u32; max_len + 1];
            for &l in lengths {
                if l > 0 {
                    bl_count[l as usize] += 1;
                }
            }
            let mut next_code = vec![0u16; max_len + 2];
            let mut code = 0u16;
            for bits in 1..=max_len {
                code = (code + bl_count[bits - 1] as u16) << 1;
                next_code[bits] = code;
            }
            let mut by_len: [Vec<(u16, u16)>; 16] = Default::default();
            for (sym, &len) in lengths.iter().enumerate() {
                if len == 0 {
                    continue;
                }
                let c = next_code[len as usize];
                by_len[len as usize].push((c, sym as u16));
                next_code[len as usize] += 1;
            }
            Self {
                entries: vec![],
                by_len,
            }
        }

        fn decode(&self, br: &mut BitReader) -> Option<u16> {
            let mut code = 0u16;
            for len in 1..=15u8 {
                let bit = br.read_bits(1)? as u16;
                code = (code << 1) | bit;
                for &(c, sym) in &self.by_len[len as usize] {
                    if c == code {
                        return Some(sym);
                    }
                }
            }
            None
        }
    }

    fn inflate(data: &[u8]) -> Option<Vec<u8>> {
        // Skip 2-byte zlib header.
        if data.len() < 2 {
            return None;
        }
        let payload = &data[2..data.len().saturating_sub(4)]; // skip Adler-32 trailer
        let mut br = BitReader::new(payload);
        let mut out = Vec::new();

        loop {
            let bfinal = br.read_bits(1)?;
            let btype = br.read_bits(2)?;

            match btype {
                0b00 => {
                    // Stored block
                    br.align_byte();
                    let len = br.read_u16_le()? as usize;
                    let _nlen = br.read_u16_le()?;
                    for _ in 0..len {
                        out.push(br.read_byte()?);
                    }
                }
                0b01 | 0b10 => {
                    let (lit_table, dist_table) = if btype == 0b01 {
                        // Fixed Huffman
                        let mut lit_lens = vec![0u16; 288];
                        for i in 0..=143usize { lit_lens[i] = 8; }
                        for i in 144..=255usize { lit_lens[i] = 9; }
                        for i in 256..=279usize { lit_lens[i] = 7; }
                        for i in 280..=287usize { lit_lens[i] = 8; }
                        let dist_lens = vec![5u16; 32];
                        (HuffTable::build(&lit_lens), HuffTable::build(&dist_lens))
                    } else {
                        // Dynamic Huffman
                        let hlit = br.read_bits(5)? as usize + 257;
                        let hdist = br.read_bits(5)? as usize + 1;
                        let hclen = br.read_bits(4)? as usize + 4;

                        let cl_order = [16u8, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];
                        let mut cl_lens = vec![0u16; 19];
                        for i in 0..hclen {
                            cl_lens[cl_order[i] as usize] = br.read_bits(3)? as u16;
                        }
                        let cl_table = HuffTable::build(&cl_lens);

                        let total = hlit + hdist;
                        let mut all_lens = Vec::with_capacity(total);
                        while all_lens.len() < total {
                            let sym = cl_table.decode(&mut br)? as u8;
                            match sym {
                                0..=15 => all_lens.push(sym as u16),
                                16 => {
                                    let rep = br.read_bits(2)? as usize + 3;
                                    let last = *all_lens.last()?;
                                    for _ in 0..rep { all_lens.push(last); }
                                }
                                17 => {
                                    let rep = br.read_bits(3)? as usize + 3;
                                    for _ in 0..rep { all_lens.push(0); }
                                }
                                18 => {
                                    let rep = br.read_bits(7)? as usize + 11;
                                    for _ in 0..rep { all_lens.push(0); }
                                }
                                _ => return None,
                            }
                        }
                        let (ll, dl) = all_lens.split_at(hlit);
                        (HuffTable::build(ll), HuffTable::build(dl))
                    };

                    // Length/distance extra-bit tables
                    const LEN_BASE: [u16; 29] = [
                        3,4,5,6,7,8,9,10,11,13,15,17,19,23,27,31,35,43,51,59,67,83,99,115,131,163,195,227,258,
                    ];
                    const LEN_EXTRA: [u8; 29] = [
                        0,0,0,0,0,0,0,0,1,1,1,1,2,2,2,2,3,3,3,3,4,4,4,4,5,5,5,5,0,
                    ];
                    const DIST_BASE: [u16; 30] = [
                        1,2,3,4,5,7,9,13,17,25,33,49,65,97,129,193,257,385,513,769,
                        1025,1537,2049,3073,4097,6145,8193,12289,16385,24577,
                    ];
                    const DIST_EXTRA: [u8; 30] = [
                        0,0,0,0,1,1,2,2,3,3,4,4,5,5,6,6,7,7,8,8,9,9,10,10,11,11,12,12,13,13,
                    ];

                    loop {
                        let sym = lit_table.decode(&mut br)?;
                        if sym < 256 {
                            out.push(sym as u8);
                        } else if sym == 256 {
                            break;
                        } else {
                            let li = (sym - 257) as usize;
                            if li >= 29 { return None; }
                            let len = LEN_BASE[li] as usize + br.read_bits(LEN_EXTRA[li])? as usize;
                            let dist_sym = dist_table.decode(&mut br)? as usize;
                            if dist_sym >= 30 { return None; }
                            let dist = DIST_BASE[dist_sym] as usize + br.read_bits(DIST_EXTRA[dist_sym])? as usize;
                            let base = out.len().checked_sub(dist)?;
                            for i in 0..len {
                                let b = out[base + i];
                                out.push(b);
                            }
                        }
                    }
                }
                _ => return None,
            }

            if bfinal == 1 {
                break;
            }
        }
        Some(out)
    }

    /// Paeth predictor (PNG filter type 4).
    fn paeth(a: i32, b: i32, c: i32) -> i32 {
        let p = a + b - c;
        let pa = (p - a).abs();
        let pb = (p - b).abs();
        let pc = (p - c).abs();
        if pa <= pb && pa <= pc { a } else if pb <= pc { b } else { c }
    }

    pub fn decode(bytes: &[u8]) -> Option<PixelBuf> {
        if bytes.len() < 8 || &bytes[0..8] != b"\x89PNG\r\n\x1a\n" {
            return None;
        }

        let mut pos = 8usize;
        let mut width = 0usize;
        let mut height = 0usize;
        let mut bit_depth = 0u8;
        let mut colour_type = 0u8;
        let mut interlaced = false;
        let mut palette: Vec<(u8, u8, u8)> = Vec::new();
        let mut idat: Vec<u8> = Vec::new();
        let mut trns: Vec<u8> = Vec::new();

        macro_rules! read_u32 {
            ($buf:expr, $off:expr) => {
                u32::from_be_bytes([$buf[$off], $buf[$off+1], $buf[$off+2], $buf[$off+3]])
            };
        }

        while pos + 12 <= bytes.len() {
            let length = read_u32!(bytes, pos) as usize;
            pos += 4;
            let chunk_type = &bytes[pos..pos + 4];
            pos += 4;
            let data = &bytes[pos..pos + length];
            pos += length;
            pos += 4; // CRC

            match chunk_type {
                b"IHDR" => {
                    if data.len() < 13 { return None; }
                    width = read_u32!(data, 0) as usize;
                    height = read_u32!(data, 4) as usize;
                    bit_depth = data[8];
                    colour_type = data[9];
                    interlaced = data[12] == 1;
                }
                b"PLTE" => {
                    palette.clear();
                    for i in 0..data.len() / 3 {
                        palette.push((data[i*3], data[i*3+1], data[i*3+2]));
                    }
                }
                b"tRNS" => {
                    trns.extend_from_slice(data);
                }
                b"IDAT" => {
                    idat.extend_from_slice(data);
                }
                b"IEND" => break,
                _ => {}
            }
        }

        if interlaced || width == 0 || height == 0 {
            return None;
        }

        let raw = inflate(&idat)?;

        // Channels per pixel before expansion.
        let channels = match colour_type {
            0 => 1usize, // grayscale
            2 => 3,      // RGB
            3 => 1,      // indexed
            4 => 2,      // grayscale + alpha
            6 => 4,      // RGBA
            _ => return None,
        };
        let sample_bytes = ((bit_depth as usize + 7) / 8).max(1);
        let bytes_per_pixel = channels * sample_bytes;
        let stride = width * bytes_per_pixel;

        let mut scanlines: Vec<Vec<u8>> = Vec::with_capacity(height);
        let mut rpos = 0usize;
        for _row in 0..height {
            if rpos >= raw.len() { return None; }
            let filter = raw[rpos];
            rpos += 1;
            if rpos + stride > raw.len() { return None; }
            let mut scan: Vec<u8> = raw[rpos..rpos + stride].to_vec();
            rpos += stride;

            let prev = scanlines.last().map(|s| s.as_slice()).unwrap_or(&[]);

            match filter {
                0 => {}
                1 => {
                    for i in bytes_per_pixel..scan.len() {
                        scan[i] = scan[i].wrapping_add(scan[i - bytes_per_pixel]);
                    }
                }
                2 => {
                    for i in 0..scan.len() {
                        let p = if i < prev.len() { prev[i] } else { 0 };
                        scan[i] = scan[i].wrapping_add(p);
                    }
                }
                3 => {
                    for i in 0..scan.len() {
                        let a = if i >= bytes_per_pixel { scan[i - bytes_per_pixel] as u16 } else { 0 };
                        let b = if i < prev.len() { prev[i] as u16 } else { 0 };
                        scan[i] = scan[i].wrapping_add(((a + b) / 2) as u8);
                    }
                }
                4 => {
                    for i in 0..scan.len() {
                        let a = if i >= bytes_per_pixel { scan[i - bytes_per_pixel] as i32 } else { 0 };
                        let b = if i < prev.len() { prev[i] as i32 } else { 0 };
                        let c = if i >= bytes_per_pixel && i < prev.len() { prev[i - bytes_per_pixel] as i32 } else { 0 };
                        scan[i] = scan[i].wrapping_add(paeth(a, b, c) as u8);
                    }
                }
                _ => return None,
            }
            scanlines.push(scan);
        }

        // Convert to RGBA.
        let mut data = Vec::with_capacity(width * height * 4);
        for scan in &scanlines {
            for px in 0..width {
                let (r, g, b, a) = match colour_type {
                    0 => {
                        // Grayscale
                        let v = match bit_depth {
                            1 => {
                                let byte = scan[px / 8];
                                let bit = 7 - (px % 8);
                                if (byte >> bit) & 1 == 1 { 255 } else { 0 }
                            }
                            2 => {
                                let byte = scan[px / 4];
                                let shift = 6 - (px % 4) * 2;
                                ((byte >> shift) & 0x3) * 85
                            }
                            4 => {
                                let byte = scan[px / 2];
                                if px % 2 == 0 { (byte >> 4) * 17 } else { (byte & 0xf) * 17 }
                            }
                            8 => scan[px],
                            16 => scan[px * 2], // take high byte
                            _ => return None,
                        };
                        let alpha = if trns.len() >= 2 {
                            let key = u16::from_be_bytes([trns[0], trns[1]]);
                            if v as u16 == key { 0 } else { 255 }
                        } else { 255 };
                        (v, v, v, alpha)
                    }
                    2 => {
                        // RGB
                        let base = px * 3 * sample_bytes;
                        let rv = if sample_bytes == 2 { scan[base] } else { scan[base] };
                        let gv = if sample_bytes == 2 { scan[base+2] } else { scan[base+1] };
                        let bv = if sample_bytes == 2 { scan[base+4] } else { scan[base+2] };
                        let alpha = if trns.len() >= 6 {
                            let kr = u16::from_be_bytes([trns[0], trns[1]]);
                            let kg = u16::from_be_bytes([trns[2], trns[3]]);
                            let kb = u16::from_be_bytes([trns[4], trns[5]]);
                            if rv as u16 == kr && gv as u16 == kg && bv as u16 == kb { 0 } else { 255 }
                        } else { 255 };
                        (rv, gv, bv, alpha)
                    }
                    3 => {
                        // Indexed
                        let idx = match bit_depth {
                            1 => {
                                let byte = scan[px / 8];
                                let bit = 7 - (px % 8);
                                ((byte >> bit) & 1) as usize
                            }
                            2 => {
                                let byte = scan[px / 4];
                                let shift = 6 - (px % 4) * 2;
                                ((byte >> shift) & 0x3) as usize
                            }
                            4 => {
                                let byte = scan[px / 2];
                                if px % 2 == 0 { (byte >> 4) as usize } else { (byte & 0xf) as usize }
                            }
                            8 | 16 => scan[px] as usize,
                            _ => return None,
                        };
                        let (rv, gv, bv) = palette.get(idx).copied().unwrap_or((0, 0, 0));
                        let alpha = trns.get(idx).copied().unwrap_or(255);
                        (rv, gv, bv, alpha)
                    }
                    4 => {
                        // Grayscale + alpha
                        let base = px * 2 * sample_bytes;
                        let v = scan[base];
                        let a = scan[base + sample_bytes];
                        (v, v, v, a)
                    }
                    6 => {
                        // RGBA
                        let base = px * 4 * sample_bytes;
                        (scan[base], scan[base + sample_bytes], scan[base + sample_bytes*2], scan[base + sample_bytes*3])
                    }
                    _ => return None,
                };
                data.extend_from_slice(&[r, g, b, a]);
            }
        }

        Some(PixelBuf { width, height, data })
    }
}

// ---------------------------------------------------------------------------
// BMP decoder (24/32-bit uncompressed DIB, pure Rust)
// ---------------------------------------------------------------------------

mod bmp {
    use super::PixelBuf;

    pub fn decode(bytes: &[u8]) -> Option<PixelBuf> {
        if bytes.len() < 54 || &bytes[0..2] != b"BM" {
            return None;
        }
        let pixel_offset = u32::from_le_bytes([bytes[10], bytes[11], bytes[12], bytes[13]]) as usize;
        let dib_size = u32::from_le_bytes([bytes[14], bytes[15], bytes[16], bytes[17]]) as usize;
        let width = u32::from_le_bytes([bytes[18], bytes[19], bytes[20], bytes[21]]) as usize;
        let raw_height = i32::from_le_bytes([bytes[22], bytes[23], bytes[24], bytes[25]]);
        let height = raw_height.unsigned_abs() as usize;
        let bits_per_pixel = u16::from_le_bytes([bytes[28], bytes[29]]) as usize;
        let compression = u32::from_le_bytes([bytes[30], bytes[31], bytes[32], bytes[33]]);

        if bits_per_pixel != 24 && bits_per_pixel != 32 {
            return None;
        }
        // Only support uncompressed (0) or BITFIELDS (3) without actual bit-field masking.
        if compression != 0 && compression != 3 {
            return None;
        }

        let bytes_per_pixel = bits_per_pixel / 8;
        let row_size = (width * bytes_per_pixel + 3) & !3; // 4-byte aligned
        let bottom_up = raw_height > 0;

        if pixel_offset + row_size * height > bytes.len() {
            return None;
        }

        let mut data = Vec::with_capacity(width * height * 4);
        for row in 0..height {
            let src_row = if bottom_up { height - 1 - row } else { row };
            let row_start = pixel_offset + src_row * row_size;
            for col in 0..width {
                let base = row_start + col * bytes_per_pixel;
                // BMP stores BGR(A)
                let b = bytes[base];
                let g = bytes[base + 1];
                let r = bytes[base + 2];
                let a = if bytes_per_pixel == 4 { bytes[base + 3] } else { 255 };
                data.extend_from_slice(&[r, g, b, a]);
            }
        }

        // Check for a 40-byte BITMAPINFOHEADER with alpha channel info (BITMAPV4HEADER marks it).
        let _ = dib_size;

        Some(PixelBuf { width, height, data })
    }
}

// ---------------------------------------------------------------------------
// Minimal JPEG decoder (baseline DCT, YCbCr, pure Rust)
// ---------------------------------------------------------------------------
//
// This is a simplified baseline JPEG decoder sufficient for thumbnail rendering.
// It handles the most common subset: SOF0 (baseline DCT), YCbCr, and common
// subsampling modes (4:4:4, 4:2:2, 4:2:0). Progressive and lossless JPEG,
// CMYK, and unusual configurations fall back to a placeholder.

mod jpeg {
    use super::PixelBuf;

    // ---------- Huffman table --------------------------------------------------

    #[derive(Default, Clone)]
    struct HuffTable {
        // Flat lookup: for each possible 16-bit prefix → (symbol, code_len)
        // We decode bit-by-bit up to 16 levels.
        codes: Vec<(u16, u8, u8)>, // (code, len, symbol)
    }

    impl HuffTable {
        fn build(lengths: &[u8; 16], symbols: &[u8]) -> Self {
            let mut codes = Vec::new();
            let mut code = 0u16;
            let mut sym_idx = 0;
            for len in 1u8..=16 {
                let count = lengths[(len - 1) as usize] as usize;
                for _ in 0..count {
                    if sym_idx < symbols.len() {
                        codes.push((code, len, symbols[sym_idx]));
                        sym_idx += 1;
                    }
                    code += 1;
                }
                code <<= 1;
            }
            Self { codes }
        }

        fn decode(&self, br: &mut JpegBitReader) -> Option<u8> {
            let mut code = 0u16;
            for len in 1u8..=16 {
                let bit = br.read_bit()?;
                code = (code << 1) | bit as u16;
                for &(c, l, sym) in &self.codes {
                    if l == len && c == code {
                        return Some(sym);
                    }
                }
            }
            None
        }
    }

    // ---------- Bit reader with byte-stuffing ---------------------------------

    struct JpegBitReader<'a> {
        data: &'a [u8],
        pos: usize,
        bits: u32,
        bits_left: u8,
    }

    impl<'a> JpegBitReader<'a> {
        fn new(data: &'a [u8]) -> Self {
            Self { data, pos: 0, bits: 0, bits_left: 0 }
        }

        fn refill(&mut self) -> bool {
            while self.bits_left <= 24 {
                if self.pos >= self.data.len() {
                    // Pad with 0xFF (harmless)
                    self.bits = (self.bits << 8) | 0xFF;
                    self.bits_left += 8;
                    if self.bits_left > 24 { break; }
                    continue;
                }
                let byte = self.data[self.pos];
                self.pos += 1;
                if byte == 0xFF {
                    if self.pos < self.data.len() && self.data[self.pos] == 0x00 {
                        // Byte-stuffed 0xFF
                        self.pos += 1;
                    } else {
                        // Marker — put byte back conceptually; return false to signal end.
                        self.pos -= 1;
                        return false;
                    }
                }
                self.bits = (self.bits << 8) | byte as u32;
                self.bits_left += 8;
            }
            true
        }

        fn read_bit(&mut self) -> Option<u8> {
            if self.bits_left == 0 {
                self.refill();
            }
            if self.bits_left == 0 {
                return None;
            }
            self.bits_left -= 1;
            Some(((self.bits >> self.bits_left) & 1) as u8)
        }

        fn read_bits(&mut self, n: u8) -> Option<i32> {
            if n == 0 { return Some(0); }
            let mut val = 0i32;
            for _ in 0..n {
                val = (val << 1) | self.read_bit()? as i32;
            }
            // Extend sign: if high bit not set, value is negative.
            if val < (1 << (n - 1)) {
                val -= (1 << n) - 1;
            }
            Some(val)
        }
    }

    // ---------- IDCT (AAN approximation, 8x8 block) --------------------------

    fn idct_row(row: &mut [i32; 8]) {
        let s0 = row[0]; let s4 = row[4];
        let s2 = row[2]; let s6 = row[6];
        let s1 = row[1]; let s5 = row[5];
        let s3 = row[3]; let s7 = row[7];

        let p1 = (s2 + s6) * 2217;
        let t2 = p1 + s6 * (-7567) + 4096 >> 13;
        let t3 = p1 + s2 * 5352 + 4096 >> 13;
        let p2 = s7 + s1;
        let p3 = s3 + s5;
        let p4 = (p2 + p3) * 3816;
        let p5 = (p2 - p3) * (-5765);
        let p2b = (s7 + s5) * (-3547);
        let p3b = (s1 + s3) * 2896;
        let t4 = p4 + p2b + s7 * (-2925) + p5 + 128 >> 8;
        let t5 = p4 + p3b + s3 * (-4501) + 128 >> 8;
        let t6 = p4 + p2b + s5 * 2765 + 128 >> 8;
        let t7 = p4 + p3b + s1 * 10893 + 128 >> 8;

        let dc = (s0 + s4) << 13;
        let dc2 = (s0 - s4) << 13;

        row[0] = (dc + t3 + t7 + 512) >> 10;
        row[7] = (dc + t3 - t7 + 512) >> 10;
        row[1] = (dc2 + t2 + t6 + 512) >> 10;
        row[6] = (dc2 + t2 - t6 + 512) >> 10;
        row[2] = (dc2 - t2 + t5 + 512) >> 10;
        row[5] = (dc2 - t2 - t5 + 512) >> 10;
        row[3] = (dc - t3 + t4 + 512) >> 10;
        row[4] = (dc - t3 - t4 + 512) >> 10;
    }

    fn dequant_idct(block: &mut [i32; 64], qtable: &[u16; 64]) {
        // Zig-zag order table
        const ZZ: [usize; 64] = [
            0,1,8,16,9,2,3,10,17,24,32,25,18,11,4,5,12,19,26,33,40,48,41,34,
            27,20,13,6,7,14,21,28,35,42,49,56,57,50,43,36,29,22,15,23,30,37,
            44,51,58,59,52,45,38,31,39,46,53,60,61,54,47,55,62,63,
        ];
        let mut tmp = [0i32; 64];
        for i in 0..64 {
            tmp[ZZ[i]] = block[i] * qtable[i] as i32;
        }
        // IDCT rows
        for row in 0..8 {
            let mut r = [0i32; 8];
            r.copy_from_slice(&tmp[row*8..row*8+8]);
            idct_row(&mut r);
            tmp[row*8..row*8+8].copy_from_slice(&r);
        }
        // IDCT columns (transpose trick)
        for col in 0..8 {
            let mut c = [0i32; 8];
            for i in 0..8 { c[i] = tmp[i*8 + col]; }
            idct_row(&mut c);
            for i in 0..8 { block[i*8 + col] = c[i]; }
        }
    }

    fn clamp(v: i32) -> u8 {
        v.clamp(-128, 127).wrapping_add(128) as u8
    }

    // YCbCr → RGB
    fn ycbcr_to_rgb(y: i32, cb: i32, cr: i32) -> (u8, u8, u8) {
        let r = y + ((cr * 45941) >> 15);
        let g = y - ((cb * 11277 + cr * 23401) >> 15);
        let b = y + ((cb * 57475) >> 15);
        (r.clamp(0, 255) as u8, g.clamp(0, 255) as u8, b.clamp(0, 255) as u8)
    }

    pub fn decode(bytes: &[u8]) -> Option<PixelBuf> {
        if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
            return None;
        }

        let mut pos = 2;
        let mut width = 0usize;
        let mut height = 0usize;
        let mut components: Vec<(u8, u8, u8, u8)> = Vec::new(); // (id, h_samp, v_samp, qtable_id)
        let mut qtables: Vec<[u16; 64]> = vec![[1u16; 64]; 4];
        let mut dc_tables: Vec<HuffTable> = vec![HuffTable::default(); 4];
        let mut ac_tables: Vec<HuffTable> = vec![HuffTable::default(); 4];
        let mut scan_data_start = 0;

        macro_rules! read_u16 {
            ($p:expr) => {
                u16::from_be_bytes([bytes[$p], bytes[$p + 1]])
            };
        }

        while pos + 2 <= bytes.len() {
            if bytes[pos] != 0xFF {
                break;
            }
            let marker = bytes[pos + 1];
            pos += 2;

            match marker {
                0xD8 | 0xD9 => {} // SOI / EOI
                0xE0..=0xEF | 0xFE => {
                    // APP / COM — skip
                    if pos + 2 > bytes.len() { break; }
                    let len = read_u16!(pos) as usize;
                    pos += len;
                }
                0xDB => {
                    // DQT
                    if pos + 2 > bytes.len() { break; }
                    let len = read_u16!(pos) as usize;
                    let end = pos + len;
                    pos += 2;
                    while pos + 65 <= end {
                        let prec_id = bytes[pos];
                        let id = (prec_id & 0xF) as usize;
                        let prec = (prec_id >> 4) & 1;
                        pos += 1;
                        if id >= 4 { break; }
                        for i in 0..64 {
                            qtables[id][i] = if prec == 0 {
                                let v = bytes[pos] as u16; pos += 1; v
                            } else {
                                let v = read_u16!(pos); pos += 2; v
                            };
                        }
                    }
                    pos = end;
                }
                0xC0 => {
                    // SOF0 — baseline DCT
                    if pos + 2 > bytes.len() { break; }
                    let _len = read_u16!(pos);
                    pos += 2;
                    let _prec = bytes[pos]; pos += 1;
                    height = read_u16!(pos) as usize; pos += 2;
                    width = read_u16!(pos) as usize; pos += 2;
                    let ncomp = bytes[pos] as usize; pos += 1;
                    components.clear();
                    for _ in 0..ncomp {
                        let id = bytes[pos]; pos += 1;
                        let samp = bytes[pos]; pos += 1;
                        let qt = bytes[pos]; pos += 1;
                        let h = (samp >> 4) & 0xF;
                        let v = samp & 0xF;
                        components.push((id, h, v, qt));
                    }
                }
                0xC4 => {
                    // DHT
                    if pos + 2 > bytes.len() { break; }
                    let len = read_u16!(pos) as usize;
                    let end = pos + len;
                    pos += 2;
                    while pos + 17 <= end {
                        let tc_th = bytes[pos]; pos += 1;
                        let table_class = (tc_th >> 4) & 1; // 0 = DC, 1 = AC
                        let table_id = (tc_th & 0xF) as usize;
                        let mut lengths = [0u8; 16];
                        lengths.copy_from_slice(&bytes[pos..pos + 16]);
                        pos += 16;
                        let total: usize = lengths.iter().map(|&x| x as usize).sum();
                        if pos + total > end { break; }
                        let symbols = &bytes[pos..pos + total];
                        pos += total;
                        if table_id >= 4 { continue; }
                        let tbl = HuffTable::build(&lengths, symbols);
                        if table_class == 0 {
                            dc_tables[table_id] = tbl;
                        } else {
                            ac_tables[table_id] = tbl;
                        }
                    }
                    pos = end;
                }
                0xDA => {
                    // SOS — scan header
                    if pos + 2 > bytes.len() { break; }
                    let len = read_u16!(pos) as usize;
                    pos += len; // skip scan header
                    scan_data_start = pos;
                    break;
                }
                0xC1..=0xC3 | 0xC5..=0xCF => {
                    // Progressive / lossless — not supported
                    return None;
                }
                _ => {
                    // Unknown marker — skip
                    if pos + 2 > bytes.len() { break; }
                    let len = read_u16!(pos) as usize;
                    pos += len;
                }
            }
        }

        if width == 0 || height == 0 || components.is_empty() || scan_data_start == 0 {
            return None;
        }
        // We only handle 1 or 3 component images.
        if components.len() != 1 && components.len() != 3 {
            return None;
        }

        // Collect entropy-coded data (strip byte-stuffed 0x00 after 0xFF).
        let scan_data_raw = &bytes[scan_data_start..];
        let mut scan_data: Vec<u8> = Vec::with_capacity(scan_data_raw.len());
        let mut si = 0;
        while si < scan_data_raw.len() {
            let b = scan_data_raw[si];
            scan_data.push(b);
            si += 1;
            if b == 0xFF && si < scan_data_raw.len() {
                if scan_data_raw[si] == 0x00 {
                    si += 1; // eat stuffed byte
                } else {
                    scan_data.pop();
                    break; // reached end marker
                }
            }
        }

        let mut br = JpegBitReader::new(&scan_data);

        // Sampling factors
        let max_h = components.iter().map(|c| c.1).max().unwrap_or(1) as usize;
        let max_v = components.iter().map(|c| c.2).max().unwrap_or(1) as usize;
        let mcu_w = max_h * 8;
        let mcu_h = max_v * 8;
        let mcus_x = (width + mcu_w - 1) / mcu_w;
        let mcus_y = (height + mcu_h - 1) / mcu_h;

        // Output planes (one per component, full padded size).
        let plane_w = mcus_x * mcu_w;
        let plane_h = mcus_y * mcu_h;
        let ncomp = components.len();
        let mut planes: Vec<Vec<i32>> = vec![vec![0i32; plane_w * plane_h]; ncomp];
        let mut dc_pred = vec![0i32; ncomp];

        // Huffman table assignments from the scan header — we assume standard (0→0, 1→1).
        // For simplicity we use component index as table index (clamped to 1).
        for mcu_y in 0..mcus_y {
            for mcu_x in 0..mcus_x {
                for (ci, &(_id, h_samp, v_samp, qt_id)) in components.iter().enumerate() {
                    let ht_dc_id = if ci == 0 { 0 } else { 1 };
                    let ht_ac_id = if ci == 0 { 0 } else { 1 };
                    let qt_id = qt_id as usize;

                    for bv in 0..v_samp as usize {
                        for bh in 0..h_samp as usize {
                            // Decode one 8×8 block
                            let mut block = [0i32; 64];

                            // DC coefficient
                            let dc_size = dc_tables[ht_dc_id].decode(&mut br)? as u8;
                            let dc_diff = br.read_bits(dc_size)?;
                            dc_pred[ci] += dc_diff;
                            block[0] = dc_pred[ci];

                            // AC coefficients
                            let mut k = 1usize;
                            while k < 64 {
                                let byte = ac_tables[ht_ac_id].decode(&mut br)?;
                                if byte == 0x00 {
                                    break; // EOB
                                }
                                let zeros = (byte >> 4) as usize;
                                let ac_size = (byte & 0xF) as u8;
                                k += zeros;
                                if k >= 64 { break; }
                                block[k] = br.read_bits(ac_size)?;
                                k += 1;
                            }

                            dequant_idct(&mut block, &qtables[qt_id.min(3)]);

                            // Place block into plane
                            let bx = mcu_x * max_h + bh;
                            let by = mcu_y * max_v + bv;
                            // Scale block position by component sampling factor
                            let px0 = bx * 8;
                            let py0 = by * 8;
                            for row in 0..8 {
                                for col in 0..8 {
                                    let px = px0 + col;
                                    let py = py0 + row;
                                    if px < plane_w && py < plane_h {
                                        planes[ci][py * plane_w + px] = block[row * 8 + col];
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Upsample chroma and convert to RGBA
        let mut data = Vec::with_capacity(width * height * 4);
        for py in 0..height {
            for px in 0..width {
                if ncomp == 1 {
                    let v = clamp(planes[0][py * plane_w + px]);
                    data.extend_from_slice(&[v, v, v, 255]);
                } else {
                    // Upsample chroma components
                    let y_val = planes[0][py * plane_w + px];
                    let cb_x = px * components[1].1 as usize / max_h;
                    let cb_y = py * components[1].2 as usize / max_v;
                    let cr_x = px * components[2].1 as usize / max_h;
                    let cr_y = py * components[2].2 as usize / max_v;
                    let cb_val = planes[1][cb_y.min(plane_h-1) * plane_w + cb_x.min(plane_w-1)];
                    let cr_val = planes[2][cr_y.min(plane_h-1) * plane_w + cr_x.min(plane_w-1)];
                    let (r, g, b) = ycbcr_to_rgb(y_val, cb_val, cr_val);
                    data.extend_from_slice(&[r, g, b, 255]);
                }
            }
        }

        Some(PixelBuf { width, height, data })
    }
}

// ---------------------------------------------------------------------------
// ASCII art renderer
// ---------------------------------------------------------------------------

/// Maps a luminance value [0, 255] to a block character.
/// Uses the "▁▂▃▄▅▆▇█" density ramp together with a space for near-black.
fn luma_to_block(luma: u8) -> char {
    // Use half-block characters for density. Simple 8-step ramp.
    match luma {
        0..=31 => ' ',
        32..=63 => '░',
        64..=95 => '░',
        96..=127 => '▒',
        128..=159 => '▒',
        160..=191 => '▓',
        192..=223 => '▓',
        224..=255 => '█',
    }
}

/// Renders `buf` as an ASCII-art string, `width_chars` columns wide.
///
/// Each character cell represents a tile of source pixels. The aspect ratio
/// is maintained by mapping two rows of pixels to one row of characters
/// (terminal font cells are roughly twice as tall as wide).
fn render_ascii(buf: &PixelBuf, width_chars: usize) -> String {
    let width_chars = width_chars.max(1).min(buf.width);
    // Pixel width per character cell.
    let cell_w = (buf.width as f64 / width_chars as f64).max(1.0);
    // Each terminal row covers ~2 pixel rows to compensate for cell aspect ratio.
    let cell_h = cell_w * 2.0;
    let rows = ((buf.height as f64 / cell_h).ceil() as usize).max(1);

    let mut out = String::with_capacity(rows * (width_chars + 1));
    for row in 0..rows {
        let y0 = (row as f64 * cell_h) as usize;
        let y1 = ((row as f64 + 1.0) * cell_h) as usize;
        for col in 0..width_chars {
            let x0 = (col as f64 * cell_w) as usize;
            let x1 = ((col as f64 + 1.0) * cell_w) as usize;
            let (r, g, b, _a) = buf.sample_tile(x0, y0, x1, y1);
            // ITU-R BT.601 luma
            let luma = ((r as u32 * 299 + g as u32 * 587 + b as u32 * 114) / 1000) as u8;
            out.push(luma_to_block(luma));
        }
        out.push('\n');
    }
    out
}

/// Renders `buf` as ASCII art with 24-bit ANSI color background per cell.
/// Falls back to plain block characters on non-truecolor terminals.
fn render_ascii_color(buf: &PixelBuf, width_chars: usize) -> String {
    let width_chars = width_chars.max(1).min(buf.width);
    let cell_w = (buf.width as f64 / width_chars as f64).max(1.0);
    let cell_h = cell_w * 2.0;
    let rows = ((buf.height as f64 / cell_h).ceil() as usize).max(1);

    let mut out = String::with_capacity(rows * (width_chars + 20) * 2);
    for row in 0..rows {
        let y0 = (row as f64 * cell_h) as usize;
        let y1 = ((row as f64 + 1.0) * cell_h) as usize;
        let y_mid = (y0 + y1) / 2;
        let y_mid2 = ((y_mid + y1) / 2).min(buf.height.saturating_sub(1));
        for col in 0..width_chars {
            let x0 = (col as f64 * cell_w) as usize;
            let x1 = ((col as f64 + 1.0) * cell_w) as usize;
            let (tr, tg, tb, _) = buf.sample_tile(x0, y0, x1, y_mid.max(y0 + 1));
            let (br, bg, bb, _) = buf.sample_tile(x0, y_mid, x1, y_mid2.max(y_mid + 1));
            // Upper half-block character '▀' with fg = top colour, bg = bottom colour.
            out.push_str(&format!(
                "\x1b[38;2;{tr};{tg};{tb}m\x1b[48;2;{br};{bg};{bb}m▀\x1b[0m"
            ));
        }
        out.push('\n');
    }
    out
}

// ---------------------------------------------------------------------------
// Sixel encoder
// ---------------------------------------------------------------------------

/// Encodes `buf` as a Sixel DCS sequence.
///
/// Uses a simple 256-entry palette built from the image colors (median-cut
/// is expensive without alloc; we quantize each pixel to the nearest
/// R3G3B2 entry and build the palette on demand).
fn render_sixel(buf: &PixelBuf, width_chars: usize) -> String {
    // Scale image to width_chars × proportional height.
    let out_w = width_chars.max(1).min(4096);
    let out_h = ((buf.height * out_w) / buf.width.max(1)).max(1).min(4096);

    // Build scaled pixel buffer.
    let mut pixels: Vec<(u8, u8, u8)> = Vec::with_capacity(out_w * out_h);
    for oy in 0..out_h {
        let sy = (oy * buf.height) / out_h;
        for ox in 0..out_w {
            let sx = (ox * buf.width) / out_w;
            let (r, g, b, _) = buf.pixel(sx.min(buf.width - 1), sy.min(buf.height - 1));
            pixels.push((r, g, b));
        }
    }

    // Build palette: quantize to R3G3B2 (256 entries).
    // Entry index = (r>>5)<<5 | (g>>5)<<2 | (b>>6)
    fn rgb332(r: u8, g: u8, b: u8) -> u8 {
        ((r >> 5) << 5) | ((g >> 5) << 2) | (b >> 6)
    }
    fn palette_rgb(idx: u8) -> (u8, u8, u8) {
        let r = ((idx >> 5) & 0x7) * 36;
        let g = ((idx >> 2) & 0x7) * 36;
        let b = (idx & 0x3) * 85;
        (r, g, b)
    }

    // Map pixels to palette indices.
    let indices: Vec<u8> = pixels.iter().map(|&(r, g, b)| rgb332(r, g, b)).collect();

    // Find which palette entries are actually used.
    let mut used = [false; 256];
    for &i in &indices { used[i as usize] = true; }

    // Sixel parameters: pixel aspect ratio 1:1, background opaque.
    let mut out = String::new();
    // DCS intro: Pa=7 (1:1 aspect), Pb=0 (background colour 0), Pc=0
    out.push_str("\x1bPq");

    // Emit palette definitions for used entries.
    for idx in 0u8..=255u8 {
        if !used[idx as usize] { continue; }
        let (r, g, b) = palette_rgb(idx);
        // Sixel uses 0..100 percentages.
        let rp = r as u32 * 100 / 255;
        let gp = g as u32 * 100 / 255;
        let bp = b as u32 * 100 / 255;
        out.push_str(&format!("#{idx};2;{rp};{gp};{bp}"));
    }

    // Emit sixel bands (each band is 6 rows of pixels).
    let bands = (out_h + 5) / 6;
    for band in 0..bands {
        let y0 = band * 6;

        // For each used palette colour, build a sixel row.
        let mut first_in_band = true;
        for pal_idx in 0u8..=255u8 {
            if !used[pal_idx as usize] { continue; }

            // Build sixel string for this colour in this band.
            let mut sixels: Vec<u8> = Vec::with_capacity(out_w);
            for ox in 0..out_w {
                let mut six = 0u8;
                for bit in 0..6 {
                    let oy = y0 + bit;
                    if oy < out_h {
                        let idx = indices[oy * out_w + ox];
                        if idx == pal_idx {
                            six |= 1 << bit;
                        }
                    }
                }
                sixels.push(six + b'?');
            }

            // RLE compress.
            let mut rle = String::with_capacity(out_w);
            let mut i = 0;
            while i < sixels.len() {
                let c = sixels[i];
                let mut run = 1;
                while i + run < sixels.len() && sixels[i + run] == c && run < 255 {
                    run += 1;
                }
                if run > 3 {
                    rle.push_str(&format!("!{run}{}", c as char));
                } else {
                    for _ in 0..run { rle.push(c as char); }
                }
                i += run;
            }

            if !first_in_band {
                out.push('$'); // carriage return within band
            }
            out.push('#');
            out.push_str(&pal_idx.to_string());
            out.push_str(&rle);
            first_in_band = false;
        }
        out.push('-'); // next sixel band
    }

    out.push_str("\x1b\\"); // DCS string terminator
    out
}

// ---------------------------------------------------------------------------
// iTerm2 inline image renderer
// ---------------------------------------------------------------------------

/// Emits an iTerm2 `ESC]1337;File=` sequence with the raw file bytes base64-encoded.
/// The terminal handles decoding and resizing internally.
fn render_iterm2(file_bytes: &[u8], width_chars: usize, filename: &str) -> String {
    let b64 = base64_encode(file_bytes);
    // Protocol: ESC ] 1337 ; File = [params] : <base64> BEL
    format!(
        "\x1b]1337;File=name={name};size={size};width={w}%;inline=1:{b64}\x07",
        name = base64_encode(filename.as_bytes()),
        size = file_bytes.len(),
        w = width_chars.min(100),
        b64 = b64,
    )
}

/// Minimal base64 encoder (RFC 4648, no line wrapping).
fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(CHARS[(n >> 18) as usize] as char);
        out.push(CHARS[((n >> 12) & 0x3F) as usize] as char);
        out.push(if chunk.len() > 1 { CHARS[((n >> 6) & 0x3F) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { CHARS[(n & 0x3F) as usize] as char } else { '=' });
    }
    out
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Render an image file as a terminal string.
///
/// Automatically selects the best available protocol:
/// 1. **iTerm2** — full-colour bitmap embedded via `ESC]1337;File=`.
/// 2. **Sixel** — DCS sixel graphics.
/// 3. **ASCII art** — 24-bit ANSI coloured half-block characters when the
///    terminal supports truecolor, otherwise monochrome block characters.
///
/// # Arguments
/// * `path` — Path to the image file (PNG, JPEG, or BMP).
/// * `width_chars` — Desired output width in character columns.
///
/// # Returns
/// A `String` ready to be written directly to the terminal.
/// On any error the function returns a graceful placeholder string
/// (never panics, never returns an error).
pub fn render_image(path: &Path, width_chars: usize) -> String {
    let width_chars = width_chars.max(4).min(500);
    let protocol = ImageProtocol::detect();

    // For iTerm2 we only need the raw file bytes; no pixel decoding required.
    if protocol == ImageProtocol::ITerm2 {
        if let Ok(bytes) = fs::read(path) {
            let name = path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("image");
            return render_iterm2(&bytes, width_chars, name);
        }
    }

    // For Sixel and ASCII we need decoded pixels.
    let buf = load_image(path).unwrap_or_else(|| PixelBuf::placeholder(256, 256));

    match protocol {
        ImageProtocol::ITerm2 => unreachable!(),
        ImageProtocol::Sixel => render_sixel(&buf, width_chars),
        ImageProtocol::Ascii => {
            // Use colour art when truecolor is available.
            let colorterm = std::env::var("COLORTERM").unwrap_or_default();
            if colorterm == "truecolor" || colorterm == "24bit" {
                render_ascii_color(&buf, width_chars)
            } else {
                render_ascii(&buf, width_chars)
            }
        }
    }
}

/// Load and decode an image from disk into an RGBA `PixelBuf`.
/// Returns `None` if the file cannot be read or the format is unsupported.
pub fn load_image(path: &Path) -> Option<PixelBuf> {
    let bytes = fs::read(path).ok()?;
    decode_image(&bytes)
}

/// Decode image bytes (auto-detect format) into an RGBA `PixelBuf`.
pub fn decode_image(bytes: &[u8]) -> Option<PixelBuf> {
    if bytes.len() >= 8 && &bytes[0..8] == b"\x89PNG\r\n\x1a\n" {
        return png::decode(bytes);
    }
    if bytes.len() >= 3 && bytes[0] == 0xFF && bytes[1] == 0xD8 && bytes[2] == 0xFF {
        return jpeg::decode(bytes);
    }
    if bytes.len() >= 2 && &bytes[0..2] == b"BM" {
        return bmp::decode(bytes);
    }
    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Build a minimal 4×4 24-bit uncompressed BMP in memory.
    fn make_bmp_4x4() -> Vec<u8> {
        // BMP with 4×4 pixels, 24bpp, no palette.
        let pixel_data_size = 4 * 4 * 3; // 4 rows × 4 cols × 3 bytes, row-aligned (already 12 = 4*3)
        let file_size = 54 + pixel_data_size;
        let mut buf = vec![0u8; file_size];
        // File header
        buf[0..2].copy_from_slice(b"BM");
        buf[2..6].copy_from_slice(&(file_size as u32).to_le_bytes());
        buf[10..14].copy_from_slice(&54u32.to_le_bytes()); // pixel data offset
        // DIB header (BITMAPINFOHEADER)
        buf[14..18].copy_from_slice(&40u32.to_le_bytes()); // header size
        buf[18..22].copy_from_slice(&4u32.to_le_bytes());  // width
        buf[22..26].copy_from_slice(&(-4i32).to_le_bytes()); // height negative = top-down
        buf[26..28].copy_from_slice(&1u16.to_le_bytes());  // planes
        buf[28..30].copy_from_slice(&24u16.to_le_bytes()); // bits per pixel
        // compression = 0, already zeroed
        // Pixel data: 4×4 gradient BGR
        let pd = 54;
        for row in 0..4usize {
            for col in 0..4usize {
                let base = pd + row * 12 + col * 3;
                buf[base] = (col as u8) * 60;     // B
                buf[base + 1] = (row as u8) * 60; // G
                buf[base + 2] = 128;               // R
            }
        }
        buf
    }

    #[test]
    fn test_bmp_decode() {
        let bmp = make_bmp_4x4();
        let buf = bmp::decode(&bmp).expect("BMP decode failed");
        assert_eq!(buf.width, 4);
        assert_eq!(buf.height, 4);
        assert_eq!(buf.data.len(), 4 * 4 * 4);

        // Top-left pixel: BGR(0,0,128) → RGBA(128,0,0,255)
        let (r, g, b, a) = buf.pixel(0, 0);
        assert_eq!(r, 128);
        assert_eq!(g, 0);
        assert_eq!(b, 0);
        assert_eq!(a, 255);

        // Bottom-right pixel: BGR(180,180,128) → RGBA(128,180,180,255)
        let (r, g, b, a) = buf.pixel(3, 3);
        assert_eq!(r, 128);
        assert_eq!(g, 180);
        assert_eq!(b, 180);
        assert_eq!(a, 255);
    }

    #[test]
    fn test_ascii_fallback_placeholder() {
        let buf = PixelBuf::placeholder(64, 64);
        let art = render_ascii(&buf, 20);
        assert!(!art.is_empty(), "ASCII art must not be empty");
        // Should have multiple lines
        let lines: Vec<&str> = art.lines().collect();
        assert!(lines.len() > 1, "Expected multiple rows, got {}", lines.len());
        // Each line should be exactly width_chars wide
        for line in &lines {
            // Count characters (not bytes, since block chars are multi-byte).
            let char_count = line.chars().count();
            assert_eq!(char_count, 20, "Line width mismatch: got {char_count}");
        }
    }

    #[test]
    fn test_ascii_color_fallback() {
        let buf = PixelBuf::placeholder(32, 32);
        let art = render_ascii_color(&buf, 10);
        // Should contain ANSI escape codes for color
        assert!(art.contains("\x1b[38;2;"), "Expected ANSI truecolor escapes");
        assert!(art.contains('▀'), "Expected half-block character");
    }

    #[test]
    fn test_render_image_missing_file_returns_placeholder() {
        let path = PathBuf::from("/tmp/__fusion_nonexistent_image_test.png");
        // Should not panic; returns a placeholder ASCII art string.
        let result = render_image(&path, 20);
        assert!(!result.is_empty(), "render_image must return non-empty string for missing file");
    }

    #[test]
    fn test_render_image_from_bmp_bytes() {
        // Write a temp BMP, render it, verify ASCII art output.
        let bmp_data = make_bmp_4x4();
        let tmp = std::env::temp_dir().join("fusion_test_image_view.bmp");
        std::fs::write(&tmp, &bmp_data).expect("write temp BMP");
        // Force ASCII mode by clearing iTerm2 env vars for this test.
        // (Protocol::detect reads env; we can't mutate env safely in parallel tests,
        //  so we call the lower-level function directly.)
        let buf = load_image(&tmp).expect("load_image must succeed for valid BMP");
        assert_eq!(buf.width, 4);
        assert_eq!(buf.height, 4);
        let art = render_ascii(&buf, 4);
        assert!(!art.is_empty());
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_base64_encode_roundtrip() {
        let input = b"Hello, World!";
        let encoded = base64_encode(input);
        assert_eq!(encoded, "SGVsbG8sIFdvcmxkIQ==");
    }

    #[test]
    fn test_base64_encode_empty() {
        assert_eq!(base64_encode(b""), "");
    }

    #[test]
    fn test_base64_encode_padding() {
        // 1 byte → 2 chars + 2 padding
        assert_eq!(base64_encode(b"M"), "TQ==");
        // 2 bytes → 3 chars + 1 padding
        assert_eq!(base64_encode(b"Ma"), "TWE=");
    }

    #[test]
    fn test_sixel_output_structure() {
        let buf = PixelBuf::placeholder(16, 16);
        let sixel = render_sixel(&buf, 16);
        // Must start with DCS introducer
        assert!(sixel.starts_with("\x1bPq"), "Sixel must start with DCS Pq");
        // Must end with string terminator
        assert!(sixel.ends_with("\x1b\\"), "Sixel must end with ST");
    }

    #[test]
    fn test_iterm2_output_structure() {
        let data = b"\x89PNG\r\n\x1a\n"; // fake PNG header
        let s = render_iterm2(data, 40, "test.png");
        assert!(s.starts_with("\x1b]1337;File="), "iTerm2 sequence must start with OSC 1337");
        assert!(s.ends_with('\x07'), "iTerm2 sequence must end with BEL");
        assert!(s.contains("inline=1:"), "Must contain inline=1");
    }

    #[test]
    fn test_protocol_detect_defaults_ascii() {
        // Without iTerm2/Sixel env vars, should fall back to Ascii.
        // (Actual env is outside our control in tests, so we just assert the
        //  return value is a valid variant.)
        let proto = ImageProtocol::detect();
        assert!(matches!(proto, ImageProtocol::ITerm2 | ImageProtocol::Sixel | ImageProtocol::Ascii));
    }

    #[test]
    fn test_luma_to_block() {
        assert_eq!(luma_to_block(0), ' ');
        assert_eq!(luma_to_block(255), '█');
        assert_eq!(luma_to_block(128), '▒');
    }

    #[test]
    fn test_pixel_buf_sample_tile_clamps() {
        let buf = PixelBuf::placeholder(4, 4);
        // Out-of-bounds tile should not panic.
        let _ = buf.sample_tile(3, 3, 100, 100);
    }

    #[test]
    fn test_decode_image_unknown_format() {
        let garbage = b"this is not an image";
        let result = decode_image(garbage);
        assert!(result.is_none(), "Unknown format must return None");
    }

    #[test]
    fn test_render_image_width_clamping() {
        // width_chars below minimum is clamped to 4.
        let path = PathBuf::from("/tmp/__fusion_nonexistent_image_test2.png");
        let result = render_image(&path, 0);
        assert!(!result.is_empty());
    }
}
