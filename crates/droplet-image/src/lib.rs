//! Pure-Rust image decoder supporting PNG, Baseline JPEG, BMP, PPM (P3/P6), and basic static SVG.

use std::fmt;

mod bmp;
mod jpeg;
mod png;
mod ppm;
mod svg;

pub const MAX_IMAGE_PIXELS: u64 = 64_000_000;

/// Decoded RGBA8 image buffer in row-major order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    /// Row-major RGBA8 (`width * height * 4` bytes).
    pub rgba: Vec<u8>,
}

/// Error returned when an image cannot be decoded or fails validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageError {
    pub message: String,
}

impl ImageError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl From<droplet_inflate::InflateError> for ImageError {
    fn from(err: droplet_inflate::InflateError) -> Self {
        Self::new(format!("PNG zlib data: {err}"))
    }
}

impl fmt::Display for ImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ImageError {}

/// Decode an image from raw bytes (PNG, Baseline JPEG, BMP, PPM P3/P6, or static SVG).
pub fn decode(bytes: &[u8]) -> Result<DecodedImage, ImageError> {
    if bytes.is_empty() {
        return Err(ImageError::new("empty image data"));
    }
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return png::decode(bytes);
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return jpeg::decode(bytes);
    }
    if bytes.starts_with(b"BM") {
        return bmp::decode(bytes);
    }
    if bytes.starts_with(b"P6") || bytes.starts_with(b"P3") {
        return ppm::decode(bytes);
    }
    if svg::looks_like_svg(bytes) {
        return svg::decode(bytes);
    }
    Err(ImageError::new("unsupported or unrecognized image format"))
}

/// Validate image dimensions against `MAX_IMAGE_PIXELS` and return the RGBA8 byte length.
pub fn validate_dimensions(width: u32, height: u32) -> Result<usize, ImageError> {
    if width == 0 || height == 0 {
        return Err(ImageError::new("image dimensions must be non-zero"));
    }
    let pixels = (width as u64)
        .checked_mul(height as u64)
        .ok_or_else(|| ImageError::new("image dimensions overflow"))?;
    if pixels > MAX_IMAGE_PIXELS {
        return Err(ImageError::new(format!(
            "image dimensions {}x{} ({} pixels) exceed limit of {}",
            width, height, pixels, MAX_IMAGE_PIXELS
        )));
    }
    let rgba_bytes = pixels
        .checked_mul(4)
        .ok_or_else(|| ImageError::new("image byte size overflow"))?;
    usize::try_from(rgba_bytes).map_err(|_| ImageError::new("image byte size exceeds address space"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::png::push_png_chunk;

    #[test]
    fn rejects_dimension_bombs_and_corrupt_inputs() {
        // PNG 100_000 x 100_000 bomb
        let mut png_bomb = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&100_000u32.to_be_bytes());
        ihdr.extend_from_slice(&100_000u32.to_be_bytes());
        ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
        push_png_chunk(&mut png_bomb, b"IHDR", &ihdr);
        push_png_chunk(&mut png_bomb, b"IEND", &[]);
        assert!(decode(&png_bomb).is_err());

        // BMP 100_000 x 100_000 bomb
        let mut bmp_bomb = vec![0u8; 54];
        bmp_bomb[0..2].copy_from_slice(b"BM");
        bmp_bomb[10..14].copy_from_slice(&54u32.to_le_bytes());
        bmp_bomb[14..18].copy_from_slice(&40u32.to_le_bytes());
        bmp_bomb[18..22].copy_from_slice(&100_000i32.to_le_bytes());
        bmp_bomb[22..26].copy_from_slice(&100_000i32.to_le_bytes());
        bmp_bomb[26..28].copy_from_slice(&1u16.to_le_bytes());
        bmp_bomb[28..30].copy_from_slice(&32u16.to_le_bytes());
        assert!(decode(&bmp_bomb).is_err());

        // PPM 100_000 x 100_000 bomb
        assert!(decode(b"P6 100000 100000 255\n").is_err());

        // SVG 100_000 x 100_000 bomb
        assert!(decode(b"<svg width=\"100000\" height=\"100000\"></svg>").is_err());

        // JPEG 10_000 x 10_000 bomb (exceeds MAX_IMAGE_PIXELS)
        let mut jpeg_bomb = vec![0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x0B, 8];
        jpeg_bomb.extend_from_slice(&10_000u16.to_be_bytes());
        jpeg_bomb.extend_from_slice(&10_000u16.to_be_bytes());
        jpeg_bomb.extend_from_slice(&[1, 1, 0x11, 0, 0xFF, 0xD9]);
        assert!(decode(&jpeg_bomb).is_err());

        // Corrupt / truncated JPEG / empty inputs
        assert!(decode(b"").is_err());
        assert!(decode(b"\xFF\xD8\xFF\xE0\x00\x10JFIF").is_err());
        assert!(decode(b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0DIHDRgarbage").is_err());
        assert!(decode(b"BMshort").is_err());
        assert!(decode(b"random unknown bytes").is_err());
    }
}
