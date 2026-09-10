//! System clipboard image extraction, caching, and placeholder formatting for Fusion CLI.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PastedImage {
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
    pub bytes_len: usize,
    pub media_type: String,
}

/// Standard optimal maximum dimension for multimodal LLM vision (Anthropic / OpenAI).
pub const MAX_IMAGE_DIMENSION: u32 = 1568;

/// Calculates new dimensions constrained to `max_dimension` while preserving aspect ratio.
pub fn clamp_dimensions(width: u32, height: u32, max_dimension: u32) -> (u32, u32) {
    if width == 0 || height == 0 {
        return (width, height);
    }
    if width <= max_dimension && height <= max_dimension {
        return (width, height);
    }

    if width >= height {
        let new_w = max_dimension;
        let new_h = ((height as f64) * (max_dimension as f64) / (width as f64)).round() as u32;
        (new_w, new_h.max(1))
    } else {
        let new_h = max_dimension;
        let new_w = ((width as f64) * (max_dimension as f64) / (height as f64)).round() as u32;
        (new_w.max(1), new_h)
    }
}

/// Encodes raw RGBA8 pixels into compressed PNG bytes.
pub fn encode_rgba_png(width: u32, height: u32, rgba_data: &[u8]) -> anyhow::Result<Vec<u8>> {
    let expected_len = (width as usize) * (height as usize) * 4;
    if rgba_data.len() != expected_len {
        anyhow::bail!(
            "RGBA buffer length mismatch: got {} bytes, expected {} ({}x{}x4)",
            rgba_data.len(),
            expected_len,
            width,
            height
        );
    }

    let img_buf = image::RgbaImage::from_raw(width, height, rgba_data.to_vec())
        .ok_or_else(|| anyhow::anyhow!("Invalid RGBA image buffer dimensions"))?;

    let (target_w, target_h) = clamp_dimensions(width, height, MAX_IMAGE_DIMENSION);
    let final_buf = if (target_w, target_h) != (width, height) {
        image::imageops::resize(
            &img_buf,
            target_w,
            target_h,
            image::imageops::FilterType::Triangle,
        )
    } else {
        img_buf
    };

    let mut png_bytes = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new_with_quality(
        &mut png_bytes,
        image::codecs::png::CompressionType::Fast,
        image::codecs::png::FilterType::NoFilter,
    );
    image::ImageEncoder::write_image(
        encoder,
        final_buf.as_raw(),
        target_w,
        target_h,
        image::ExtendedColorType::Rgba8,
    )?;
    Ok(png_bytes)
}

/// Formats the inline text placeholder for an attached image.
pub fn format_image_placeholder(index: usize, width: u32, height: u32) -> String {
    format!("[Image #{}, {}x{}]", index, width, height)
}

/// Returns the cache directory for pasted images (`.fusion/cache/images/`).
pub fn get_image_cache_dir() -> PathBuf {
    let dir = PathBuf::from(".fusion/cache/images");
    let _ = fs::create_dir_all(&dir);
    dir
}

/// Queries the OS clipboard for image data. If an image is found,
/// it is encoded as a PNG and saved to the local image cache.
pub fn read_clipboard_image() -> Option<PastedImage> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut clipboard = arboard::Clipboard::new().ok()?;
        let img = clipboard.get_image().ok()?;
        let raw_w = img.width as u32;
        let raw_h = img.height as u32;
        if raw_w == 0 || raw_h == 0 {
            return None;
        }

        let png_bytes = encode_rgba_png(raw_w, raw_h, &img.bytes).ok()?;
        let (width, height) = clamp_dimensions(raw_w, raw_h, MAX_IMAGE_DIMENSION);

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let cache_dir = get_image_cache_dir();
        let filename = format!("clip_{}_{}x{}.png", now, width, height);
        let path = cache_dir.join(filename);

        fs::write(&path, &png_bytes).ok()?;

        Some(PastedImage {
            path,
            width,
            height,
            bytes_len: png_bytes.len(),
            media_type: "image/png".to_string(),
        })
    }
    #[cfg(target_arch = "wasm32")]
    {
        None
    }
}

/// Loads an existing image file from disk and returns its metadata as a `PastedImage`.
pub fn load_and_cache_image_file(source_path: &Path) -> anyhow::Result<PastedImage> {
    if !source_path.exists() {
        anyhow::bail!("Image file not found: {}", source_path.display());
    }

    let bytes = fs::read(source_path)?;
    let img = image::load_from_memory(&bytes)?;
    let width = img.width();
    let height = img.height();

    let ext = source_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("png")
        .to_lowercase();
    let media_type = match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        _ => "image/png",
    }
    .to_string();

    let (target_w, target_h) = clamp_dimensions(width, height, MAX_IMAGE_DIMENSION);
    Ok(PastedImage {
        path: source_path.to_path_buf(),
        width: target_w,
        height: target_h,
        bytes_len: bytes.len(),
        media_type,
    })
}

/// Reads an image file from disk and constructs an `ImageAttachment` with base64 data.
pub fn create_image_attachment_from_file(
    path: &Path,
) -> anyhow::Result<crate::provider::types::ImageAttachment> {
    let bytes = fs::read(path)?;
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("png")
        .to_lowercase();
    let media_type = match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        _ => "image/png",
    }
    .to_string();

    use base64::Engine;
    if let Ok(img) = image::load_from_memory(&bytes) {
        let (orig_w, orig_h) = (img.width(), img.height());
        let (target_w, target_h) = clamp_dimensions(orig_w, orig_h, MAX_IMAGE_DIMENSION);
        let final_bytes = if (target_w, target_h) != (orig_w, orig_h) {
            let resized = img.resize(target_w, target_h, image::imageops::FilterType::Triangle);
            let mut out_bytes = Vec::new();
            let encoder = image::codecs::png::PngEncoder::new_with_quality(
                &mut out_bytes,
                image::codecs::png::CompressionType::Fast,
                image::codecs::png::FilterType::NoFilter,
            );
            image::ImageEncoder::write_image(
                encoder,
                resized.to_rgba8().as_raw(),
                target_w,
                target_h,
                image::ExtendedColorType::Rgba8,
            )?;
            out_bytes
        } else {
            bytes
        };
        let base64_data = base64::engine::general_purpose::STANDARD.encode(&final_bytes);
        Ok(crate::provider::types::ImageAttachment {
            media_type,
            data: base64_data,
            path: Some(path.to_string_lossy().to_string()),
            width: Some(target_w),
            height: Some(target_h),
        })
    } else {
        let base64_data = base64::engine::general_purpose::STANDARD.encode(&bytes);
        Ok(crate::provider::types::ImageAttachment {
            media_type,
            data: base64_data,
            path: Some(path.to_string_lossy().to_string()),
            width: None,
            height: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_rgba_to_png() {
        // 2x2 RGBA test image: red, green, blue, white
        let rgba_data = vec![
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
        ];
        let png_bytes = encode_rgba_png(2, 2, &rgba_data).expect("must encode png");
        assert!(!png_bytes.is_empty());
        assert_eq!(&png_bytes[0..8], b"\x89PNG\r\n\x1a\n");
    }

    #[test]
    fn test_clamp_dimensions_retina() {
        let (w, h) = clamp_dimensions(3212, 2124, 1568);
        assert_eq!(w, 1568);
        assert_eq!(h, 1037);
    }

    #[test]
    fn test_clamp_dimensions_small() {
        let (w, h) = clamp_dimensions(800, 600, 1568);
        assert_eq!(w, 800);
        assert_eq!(h, 600);
    }

    #[test]
    fn test_format_placeholder() {
        let placeholder = format_image_placeholder(1, 1920, 1080);
        assert_eq!(placeholder, "[Image #1, 1920x1080]");
    }
    #[test]
    fn test_encode_rgba_invalid_dimensions() {
        let rgba_data = vec![255, 0, 0, 255]; // Only 1 pixel
        let res = encode_rgba_png(2, 2, &rgba_data); // Expects 4 pixels
        assert!(res.is_err());
    }
}
