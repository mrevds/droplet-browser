//! Pure-Rust image decoder supporting PNG, BMP, PPM (P3/P6), and basic static SVG.

use std::fmt;

const MAX_IMAGE_PIXELS: u64 = 64_000_000;

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
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ImageError {}

/// Decode an image from raw bytes (PNG, BMP, PPM P3/P6, or static SVG).
pub fn decode(bytes: &[u8]) -> Result<DecodedImage, ImageError> {
    if bytes.is_empty() {
        return Err(ImageError::new("empty image data"));
    }
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return decode_png(bytes);
    }
    if bytes.starts_with(b"BM") {
        return decode_bmp(bytes);
    }
    if bytes.starts_with(b"P6") || bytes.starts_with(b"P3") {
        return decode_ppm(bytes);
    }
    if looks_like_svg(bytes) {
        return decode_svg(bytes);
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Err(ImageError::new("unsupported image format: JPEG"));
    }
    Err(ImageError::new("unsupported or unrecognized image format"))
}

fn validate_dimensions(width: u32, height: u32) -> Result<usize, ImageError> {
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

fn looks_like_svg(bytes: &[u8]) -> bool {
    let trimmed = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let Ok(text) = std::str::from_utf8(trimmed) else {
        return false;
    };
    let s = skip_xml_preamble(text);
    starts_with_ascii_ci(s, "<svg")
        && s.as_bytes()
            .get(4)
            .is_some_and(|b| b.is_ascii_whitespace() || *b == b'>' || *b == b'/')
}

fn starts_with_ascii_ci(s: &str, prefix: &str) -> bool {
    s.get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
}

fn find_ascii_ci(haystack: &str, needle: &str) -> Option<usize> {
    let n = needle.len();
    if n == 0 {
        return Some(0);
    }
    if haystack.len() < n {
        return None;
    }
    let hb = haystack.as_bytes();
    let nb = needle.as_bytes();
    for i in 0..=(hb.len() - n) {
        if hb[i..i + n].eq_ignore_ascii_case(nb) {
            return Some(i);
        }
    }
    None
}

fn skip_xml_preamble(mut s: &str) -> &str {
    loop {
        s = s.trim_start();
        if s.starts_with("<?") {
            if let Some(end) = s.find("?>") {
                s = &s[end + 2..];
                continue;
            }
            return "";
        }
        if s.starts_with("<!--") {
            if let Some(end) = s.find("-->") {
                s = &s[end + 3..];
                continue;
            }
            return "";
        }
        if starts_with_ascii_ci(s, "<!DOCTYPE") {
            if let Some(end) = s.find('>') {
                s = &s[end + 1..];
                continue;
            }
            return "";
        }
        return s;
    }
}

// ============================================================================
// CRC32 & Adler32
// ============================================================================

fn crc32(parts: &[&[u8]]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for part in parts {
        for &byte in *part {
            crc ^= byte as u32;
            for _ in 0..8 {
                let mask = (crc & 1).wrapping_neg();
                crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
            }
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    const MOD_ADLER: u32 = 65521;
    let mut s1: u32 = 1;
    let mut s2: u32 = 0;
    for chunk in data.chunks(5552) {
        for &b in chunk {
            s1 += b as u32;
            s2 += s1;
        }
        s1 %= MOD_ADLER;
        s2 %= MOD_ADLER;
    }
    (s2 << 16) | s1
}

// ============================================================================
// PNG Decoder (Chunks + RFC 1950/1951 Inflate + Scanline Unfiltering)
// ============================================================================

fn decode_png(bytes: &[u8]) -> Result<DecodedImage, ImageError> {
    const SIG: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if !bytes.starts_with(SIG) {
        return Err(ImageError::new("invalid PNG signature"));
    }

    let mut pos = 8usize;
    let mut ihdr: Option<PngHeader> = None;
    let mut palette: Vec<[u8; 3]> = Vec::new();
    let mut trns: Option<Vec<u8>> = None;
    let mut idat: Vec<u8> = Vec::new();
    let mut seen_iend = false;

    while pos < bytes.len() {
        if bytes.len() - pos < 12 {
            return Err(ImageError::new("truncated PNG chunk header"));
        }
        let chunk_len = u32::from_be_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]])
            as usize;
        let chunk_type: [u8; 4] = [bytes[pos + 4], bytes[pos + 5], bytes[pos + 6], bytes[pos + 7]];
        let data_start = pos + 8;
        let data_end = data_start
            .checked_add(chunk_len)
            .ok_or_else(|| ImageError::new("PNG chunk length overflow"))?;
        let crc_end = data_end
            .checked_add(4)
            .ok_or_else(|| ImageError::new("PNG chunk CRC overflow"))?;
        if crc_end > bytes.len() {
            return Err(ImageError::new("truncated PNG chunk payload"));
        }
        let payload = &bytes[data_start..data_end];
        let expected_crc = u32::from_be_bytes([
            bytes[data_end],
            bytes[data_end + 1],
            bytes[data_end + 2],
            bytes[data_end + 3],
        ]);
        let is_critical = (chunk_type[0] & 0x20) == 0;
        if is_critical || &chunk_type == b"tRNS" {
            let actual_crc = crc32(&[&chunk_type, payload]);
            if actual_crc != expected_crc {
                return Err(ImageError::new(format!(
                    "PNG chunk {:?} CRC mismatch",
                    String::from_utf8_lossy(&chunk_type)
                )));
            }
        }

        pos = crc_end;

        match &chunk_type {
            b"IHDR" => {
                if ihdr.is_some() {
                    return Err(ImageError::new("duplicate PNG IHDR chunk"));
                }
                ihdr = Some(parse_ihdr(payload)?);
            }
            b"PLTE" => {
                if ihdr.is_none() {
                    return Err(ImageError::new("PNG PLTE before IHDR"));
                }
                if payload.is_empty() || !payload.len().is_multiple_of(3) || payload.len() > 256 * 3 {
                    return Err(ImageError::new("invalid PNG PLTE chunk length"));
                }
                palette.clear();
                for rgb in payload.as_chunks::<3>().0 {
                    palette.push([rgb[0], rgb[1], rgb[2]]);
                }
            }
            b"tRNS" => {
                if ihdr.is_none() {
                    return Err(ImageError::new("PNG tRNS before IHDR"));
                }
                trns = Some(payload.to_vec());
            }
            b"IDAT" => {
                if ihdr.is_none() {
                    return Err(ImageError::new("PNG IDAT before IHDR"));
                }
                idat.extend_from_slice(payload);
            }
            b"IEND" => {
                if !payload.is_empty() {
                    return Err(ImageError::new("non-empty PNG IEND chunk"));
                }
                seen_iend = true;
                break;
            }
            _ => {
                if is_critical {
                    return Err(ImageError::new(format!(
                        "unsupported critical PNG chunk {:?}",
                        String::from_utf8_lossy(&chunk_type)
                    )));
                }
            }
        }
    }

    let hdr = ihdr.ok_or_else(|| ImageError::new("missing PNG IHDR chunk"))?;
    if !seen_iend {
        return Err(ImageError::new("missing PNG IEND chunk"));
    }
    if idat.is_empty() {
        return Err(ImageError::new("missing PNG IDAT data"));
    }
    if hdr.color_type == 3 && palette.is_empty() {
        return Err(ImageError::new("indexed PNG missing PLTE chunk"));
    }

    let rgba_len = validate_dimensions(hdr.width, hdr.height)?;
    let expected_raw_len = hdr.expected_raw_byte_len()?;
    let raw = inflate_zlib(&idat, expected_raw_len)?;
    if raw.len() < expected_raw_len {
        return Err(ImageError::new("truncated decompressed PNG image data"));
    }

    let mut rgba = vec![0u8; rgba_len];
    if hdr.interlace == 0 {
        reconstruct_png_pass(
            &raw,
            &hdr,
            hdr.width,
            hdr.height,
            0,
            0,
            1,
            1,
            &palette,
            trns.as_deref(),
            &mut rgba,
        )?;
    } else {
        const ADAM7: [(u32, u32, u32, u32); 7] = [
            (0, 0, 8, 8),
            (4, 0, 8, 8),
            (0, 4, 4, 8),
            (2, 0, 4, 4),
            (0, 2, 2, 4),
            (1, 0, 2, 2),
            (0, 1, 1, 2),
        ];
        let mut offset = 0usize;
        for &(x0, y0, dx, dy) in &ADAM7 {
            if hdr.width <= x0 || hdr.height <= y0 {
                continue;
            }
            let pass_w = (hdr.width - x0).div_ceil(dx);
            let pass_h = (hdr.height - y0).div_ceil(dy);
            let pass_bytes = hdr.pass_raw_byte_len(pass_w, pass_h)?;
            if offset + pass_bytes > raw.len() {
                return Err(ImageError::new("truncated Adam7 PNG pass data"));
            }
            reconstruct_png_pass(
                &raw[offset..offset + pass_bytes],
                &hdr,
                pass_w,
                pass_h,
                x0,
                y0,
                dx,
                dy,
                &palette,
                trns.as_deref(),
                &mut rgba,
            )?;
            offset += pass_bytes;
        }
    }

    Ok(DecodedImage {
        width: hdr.width,
        height: hdr.height,
        rgba,
    })
}

#[derive(Debug, Clone, Copy)]
struct PngHeader {
    width: u32,
    height: u32,
    bit_depth: u8,
    color_type: u8,
    interlace: u8,
}

impl PngHeader {
    fn channels(&self) -> usize {
        match self.color_type {
            0 | 3 => 1,
            2 => 3,
            4 => 2,
            6 => 4,
            _ => 0,
        }
    }

    fn filter_bpp(&self) -> usize {
        let bits = self.channels() * self.bit_depth as usize;
        bits.div_ceil(8).max(1)
    }

    fn pass_row_bytes(&self, pass_w: u32) -> Result<usize, ImageError> {
        let bits = (pass_w as usize)
            .checked_mul(self.channels())
            .and_then(|v| v.checked_mul(self.bit_depth as usize))
            .ok_or_else(|| ImageError::new("PNG scanline width overflow"))?;
        Ok(bits.div_ceil(8))
    }

    fn pass_raw_byte_len(&self, pass_w: u32, pass_h: u32) -> Result<usize, ImageError> {
        let row_bytes = self.pass_row_bytes(pass_w)?;
        (pass_h as usize)
            .checked_mul(row_bytes + 1)
            .ok_or_else(|| ImageError::new("PNG raw data length overflow"))
    }

    fn expected_raw_byte_len(&self) -> Result<usize, ImageError> {
        if self.interlace == 0 {
            return self.pass_raw_byte_len(self.width, self.height);
        }
        const ADAM7: [(u32, u32, u32, u32); 7] = [
            (0, 0, 8, 8),
            (4, 0, 8, 8),
            (0, 4, 4, 8),
            (2, 0, 4, 4),
            (0, 2, 2, 4),
            (1, 0, 2, 2),
            (0, 1, 1, 2),
        ];
        let mut total = 0usize;
        for &(x0, y0, dx, dy) in &ADAM7 {
            if self.width <= x0 || self.height <= y0 {
                continue;
            }
            let pw = (self.width - x0).div_ceil(dx);
            let ph = (self.height - y0).div_ceil(dy);
            total = total
                .checked_add(self.pass_raw_byte_len(pw, ph)?)
                .ok_or_else(|| ImageError::new("PNG Adam7 total length overflow"))?;
        }
        Ok(total)
    }
}

fn parse_ihdr(payload: &[u8]) -> Result<PngHeader, ImageError> {
    if payload.len() != 13 {
        return Err(ImageError::new("invalid PNG IHDR length"));
    }
    let width = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
    let height = u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]);
    validate_dimensions(width, height)?;
    let bit_depth = payload[8];
    let color_type = payload[9];
    let compression = payload[10];
    let filter_method = payload[11];
    let interlace = payload[12];

    if compression != 0 {
        return Err(ImageError::new("unsupported PNG compression method"));
    }
    if filter_method != 0 {
        return Err(ImageError::new("unsupported PNG filter method"));
    }
    if interlace > 1 {
        return Err(ImageError::new("unsupported PNG interlace method"));
    }

    let valid_combo = match color_type {
        0 => matches!(bit_depth, 1 | 2 | 4 | 8 | 16),
        2 | 4 | 6 => matches!(bit_depth, 8 | 16),
        3 => matches!(bit_depth, 1 | 2 | 4 | 8),
        _ => false,
    };
    if !valid_combo {
        return Err(ImageError::new(format!(
            "unsupported PNG color_type={} bit_depth={}",
            color_type, bit_depth
        )));
    }

    Ok(PngHeader {
        width,
        height,
        bit_depth,
        color_type,
        interlace,
    })
}

fn paeth_predictor(a: u8, b: u8, c: u8) -> u8 {
    let a = a as i32;
    let b = b as i32;
    let c = c as i32;
    let p = a + b - c;
    let pa = (p - a).abs();
    let pb = (p - b).abs();
    let pc = (p - c).abs();
    if pa <= pb && pa <= pc {
        a as u8
    } else if pb <= pc {
        b as u8
    } else {
        c as u8
    }
}

#[allow(clippy::too_many_arguments)]
fn reconstruct_png_pass(
    raw: &[u8],
    hdr: &PngHeader,
    pass_w: u32,
    pass_h: u32,
    x0: u32,
    y0: u32,
    dx: u32,
    dy: u32,
    palette: &[[u8; 3]],
    trns: Option<&[u8]>,
    out_rgba: &mut [u8],
) -> Result<(), ImageError> {
    let row_bytes = hdr.pass_row_bytes(pass_w)?;
    let bpp = hdr.filter_bpp();
    let mut prev_row = vec![0u8; row_bytes];
    let mut curr_row = vec![0u8; row_bytes];

    for py in 0..pass_h {
        let row_start = (py as usize) * (row_bytes + 1);
        if row_start + 1 + row_bytes > raw.len() {
            return Err(ImageError::new("truncated PNG scanline"));
        }
        let filter_type = raw[row_start];
        let src = &raw[row_start + 1..row_start + 1 + row_bytes];

        match filter_type {
            0 => curr_row.copy_from_slice(src),
            1 => {
                for i in 0..row_bytes {
                    let a = if i >= bpp { curr_row[i - bpp] } else { 0 };
                    curr_row[i] = src[i].wrapping_add(a);
                }
            }
            2 => {
                for i in 0..row_bytes {
                    curr_row[i] = src[i].wrapping_add(prev_row[i]);
                }
            }
            3 => {
                for i in 0..row_bytes {
                    let a = if i >= bpp { curr_row[i - bpp] } else { 0 };
                    let b = prev_row[i];
                    let avg = ((a as u16 + b as u16) >> 1) as u8;
                    curr_row[i] = src[i].wrapping_add(avg);
                }
            }
            4 => {
                for i in 0..row_bytes {
                    let a = if i >= bpp { curr_row[i - bpp] } else { 0 };
                    let b = prev_row[i];
                    let c = if i >= bpp { prev_row[i - bpp] } else { 0 };
                    curr_row[i] = src[i].wrapping_add(paeth_predictor(a, b, c));
                }
            }
            other => {
                return Err(ImageError::new(format!(
                    "unsupported PNG scanline filter type {}",
                    other
                )));
            }
        }

        let dst_y = (y0 + py * dy) as usize;
        write_png_scanline(
            &curr_row,
            hdr,
            pass_w,
            x0,
            dx,
            dst_y,
            palette,
            trns,
            out_rgba,
        )?;
        std::mem::swap(&mut prev_row, &mut curr_row);
    }
    Ok(())
}

fn scale_sample_to_u8(val: u16, bit_depth: u8) -> u8 {
    match bit_depth {
        1 => (val as u8) * 255,
        2 => (val as u8) * 85,
        4 => (val as u8) * 17,
        8 => val as u8,
        16 => (val >> 8) as u8,
        _ => 0,
    }
}

fn read_packed_sample(row: &[u8], idx: usize, bit_depth: u8) -> u16 {
    match bit_depth {
        1 => ((row[idx >> 3] >> (7 - (idx & 7))) & 0x01) as u16,
        2 => ((row[idx >> 2] >> (6 - ((idx & 3) << 1))) & 0x03) as u16,
        4 => ((row[idx >> 1] >> (4 - ((idx & 1) << 2))) & 0x0F) as u16,
        8 => row[idx] as u16,
        16 => u16::from_be_bytes([row[idx * 2], row[idx * 2 + 1]]),
        _ => 0,
    }
}

#[allow(clippy::too_many_arguments)]
fn write_png_scanline(
    row: &[u8],
    hdr: &PngHeader,
    pass_w: u32,
    x0: u32,
    dx: u32,
    dst_y: usize,
    palette: &[[u8; 3]],
    trns: Option<&[u8]>,
    out_rgba: &mut [u8],
) -> Result<(), ImageError> {
    let img_w = hdr.width as usize;
    let bd = hdr.bit_depth;

    let trns_gray = if hdr.color_type == 0 {
        trns.and_then(|t| (t.len() >= 2).then(|| u16::from_be_bytes([t[0], t[1]])))
    } else {
        None
    };
    let trns_rgb = if hdr.color_type == 2 {
        trns.and_then(|t| {
            (t.len() >= 6).then(|| {
                (
                    u16::from_be_bytes([t[0], t[1]]),
                    u16::from_be_bytes([t[2], t[3]]),
                    u16::from_be_bytes([t[4], t[5]]),
                )
            })
        })
    } else {
        None
    };

    for px in 0..pass_w {
        let dst_x = (x0 + px * dx) as usize;
        let out_idx = (dst_y * img_w + dst_x) * 4;
        let px_usize = px as usize;

        let rgba = match hdr.color_type {
            0 => {
                let raw_g = read_packed_sample(row, px_usize, bd);
                let g = scale_sample_to_u8(raw_g, bd);
                let a = if trns_gray == Some(raw_g) { 0 } else { 255 };
                [g, g, g, a]
            }
            2 => {
                let base = px_usize * 3;
                let rr = read_packed_sample(row, base, bd);
                let gg = read_packed_sample(row, base + 1, bd);
                let bb = read_packed_sample(row, base + 2, bd);
                let a = if trns_rgb == Some((rr, gg, bb)) {
                    0
                } else {
                    255
                };
                [
                    scale_sample_to_u8(rr, bd),
                    scale_sample_to_u8(gg, bd),
                    scale_sample_to_u8(bb, bd),
                    a,
                ]
            }
            3 => {
                let pal_idx = read_packed_sample(row, px_usize, bd) as usize;
                let rgb = *palette
                    .get(pal_idx)
                    .ok_or_else(|| ImageError::new("PNG palette index out of range"))?;
                let a = trns.and_then(|t| t.get(pal_idx).copied()).unwrap_or(255);
                [rgb[0], rgb[1], rgb[2], a]
            }
            4 => {
                let base = px_usize * 2;
                let g = scale_sample_to_u8(read_packed_sample(row, base, bd), bd);
                let a = scale_sample_to_u8(read_packed_sample(row, base + 1, bd), bd);
                [g, g, g, a]
            }
            6 => {
                let base = px_usize * 4;
                let r = scale_sample_to_u8(read_packed_sample(row, base, bd), bd);
                let g = scale_sample_to_u8(read_packed_sample(row, base + 1, bd), bd);
                let b = scale_sample_to_u8(read_packed_sample(row, base + 2, bd), bd);
                let a = scale_sample_to_u8(read_packed_sample(row, base + 3, bd), bd);
                [r, g, b, a]
            }
            _ => unreachable!(),
        };

        out_rgba[out_idx..out_idx + 4].copy_from_slice(&rgba);
    }
    Ok(())
}

// ============================================================================
// RFC 1950 Zlib + RFC 1951 Deflate Decompressor
// ============================================================================

struct BitReader<'a> {
    data: &'a [u8],
    byte_pos: usize,
    bit_buf: u32,
    bit_cnt: u8,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            byte_pos: 0,
            bit_buf: 0,
            bit_cnt: 0,
        }
    }

    fn ensure_bits(&mut self, n: u8) -> Result<(), ImageError> {
        while self.bit_cnt < n {
            if self.byte_pos >= self.data.len() {
                return Err(ImageError::new("unexpected end of deflate stream"));
            }
            self.bit_buf |= (self.data[self.byte_pos] as u32) << self.bit_cnt;
            self.byte_pos += 1;
            self.bit_cnt += 8;
        }
        Ok(())
    }

    fn read_bits(&mut self, n: u8) -> Result<u32, ImageError> {
        if n == 0 {
            return Ok(0);
        }
        self.ensure_bits(n)?;
        let val = self.bit_buf & ((1u32 << n) - 1);
        self.bit_buf >>= n;
        self.bit_cnt -= n;
        Ok(val)
    }

    fn align_to_byte(&mut self) {
        let discard = self.bit_cnt & 7;
        self.bit_buf >>= discard;
        self.bit_cnt -= discard;
    }

    fn read_aligned_u16_le(&mut self) -> Result<u16, ImageError> {
        self.align_to_byte();
        let b0 = self.read_bits(8)? as u8;
        let b1 = self.read_bits(8)? as u8;
        Ok(u16::from_le_bytes([b0, b1]))
    }

    fn read_aligned_bytes(&mut self, len: usize, out: &mut Vec<u8>) -> Result<(), ImageError> {
        self.align_to_byte();
        // Drain any whole buffered bytes first, then copy directly from slice.
        let mut remaining = len;
        while remaining > 0 && self.bit_cnt >= 8 {
            out.push((self.bit_buf & 0xFF) as u8);
            self.bit_buf >>= 8;
            self.bit_cnt -= 8;
            remaining -= 1;
        }
        if remaining > 0 {
            let end = self
                .byte_pos
                .checked_add(remaining)
                .ok_or_else(|| ImageError::new("deflate stored block overflow"))?;
            if end > self.data.len() {
                return Err(ImageError::new("truncated deflate stored block"));
            }
            out.extend_from_slice(&self.data[self.byte_pos..end]);
            self.byte_pos = end;
        }
        Ok(())
    }

    fn finish_aligned_byte_pos(&mut self) -> usize {
        self.align_to_byte();
        let buffered_bytes = (self.bit_cnt / 8) as usize;
        self.byte_pos.saturating_sub(buffered_bytes)
    }
}

const MAX_HUFFMAN_BITS: usize = 15;

struct HuffmanTree {
    counts: [u16; MAX_HUFFMAN_BITS + 1],
    symbols: Vec<u16>,
}

impl HuffmanTree {
    fn from_lengths(lengths: &[u8]) -> Result<Self, ImageError> {
        let mut counts = [0u16; MAX_HUFFMAN_BITS + 1];
        for &len in lengths {
            if len as usize > MAX_HUFFMAN_BITS {
                return Err(ImageError::new("invalid Huffman code length"));
            }
            counts[len as usize] += 1;
        }

        if counts[0] as usize == lengths.len() {
            return Ok(Self {
                counts,
                symbols: Vec::new(),
            });
        }

        let mut left: i32 = 1;
        for &count in counts.iter().take(MAX_HUFFMAN_BITS + 1).skip(1) {
            left <<= 1;
            left -= count as i32;
            if left < 0 {
                return Err(ImageError::new("oversubscribed Huffman code tree"));
            }
        }

        let mut offsets = [0u16; MAX_HUFFMAN_BITS + 1];
        for bits in 1..MAX_HUFFMAN_BITS {
            offsets[bits + 1] = offsets[bits] + counts[bits];
        }

        let total_used = (offsets[MAX_HUFFMAN_BITS] + counts[MAX_HUFFMAN_BITS]) as usize;
        let mut symbols = vec![0u16; total_used];
        for (sym, &len) in lengths.iter().enumerate() {
            if len != 0 {
                let idx = offsets[len as usize] as usize;
                offsets[len as usize] += 1;
                symbols[idx] = sym as u16;
            }
        }

        Ok(Self { counts, symbols })
    }

    fn decode(&self, reader: &mut BitReader<'_>) -> Result<u16, ImageError> {
        let mut code: i32 = 0;
        let mut first: i32 = 0;
        let mut index: usize = 0;
        for len in 1..=MAX_HUFFMAN_BITS {
            code |= reader.read_bits(1)? as i32;
            let count = self.counts[len] as i32;
            if code - count < first {
                return Ok(self.symbols[index + (code - first) as usize]);
            }
            index += count as usize;
            first += count;
            first <<= 1;
            code <<= 1;
        }
        Err(ImageError::new("invalid Huffman code in deflate stream"))
    }
}

fn fixed_huffman_trees() -> Result<(HuffmanTree, HuffmanTree), ImageError> {
    let mut lit_lengths = [0u8; 288];
    lit_lengths[..=143].fill(8);
    lit_lengths[144..=255].fill(9);
    lit_lengths[256..=279].fill(7);
    lit_lengths[280..=287].fill(8);
    let dist_lengths = [5u8; 32];
    Ok((
        HuffmanTree::from_lengths(&lit_lengths)?,
        HuffmanTree::from_lengths(&dist_lengths)?,
    ))
}

fn decode_dynamic_trees(reader: &mut BitReader<'_>) -> Result<(HuffmanTree, HuffmanTree), ImageError> {
    const CL_ORDER: [usize; 19] = [
        16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
    ];
    let hlit = reader.read_bits(5)? as usize + 257;
    let hdist = reader.read_bits(5)? as usize + 1;
    let hclen = reader.read_bits(4)? as usize + 4;
    if hlit > 286 || hdist > 30 {
        return Err(ImageError::new("invalid dynamic Huffman header counts"));
    }

    let mut cl_lengths = [0u8; 19];
    for i in 0..hclen {
        cl_lengths[CL_ORDER[i]] = reader.read_bits(3)? as u8;
    }
    let cl_tree = HuffmanTree::from_lengths(&cl_lengths)?;

    let total = hlit + hdist;
    let mut lengths = Vec::with_capacity(total);
    while lengths.len() < total {
        let sym = cl_tree.decode(reader)?;
        match sym {
            0..=15 => lengths.push(sym as u8),
            16 => {
                let prev = *lengths
                    .last()
                    .ok_or_else(|| ImageError::new("invalid repeat code in Huffman lengths"))?;
                let rep = reader.read_bits(2)? as usize + 3;
                if lengths.len() + rep > total {
                    return Err(ImageError::new("Huffman code lengths overflow"));
                }
                lengths.resize(lengths.len() + rep, prev);
            }
            17 => {
                let rep = reader.read_bits(3)? as usize + 3;
                if lengths.len() + rep > total {
                    return Err(ImageError::new("Huffman zero-repeat lengths overflow"));
                }
                lengths.resize(lengths.len() + rep, 0);
            }
            18 => {
                let rep = reader.read_bits(7)? as usize + 11;
                if lengths.len() + rep > total {
                    return Err(ImageError::new("Huffman long-zero-repeat lengths overflow"));
                }
                lengths.resize(lengths.len() + rep, 0);
            }
            _ => return Err(ImageError::new("invalid code-length Huffman symbol")),
        }
    }

    if lengths[256] == 0 {
        return Err(ImageError::new("dynamic Huffman missing EOB symbol 256"));
    }

    let lit_tree = HuffmanTree::from_lengths(&lengths[..hlit])?;
    let dist_tree = HuffmanTree::from_lengths(&lengths[hlit..])?;
    Ok((lit_tree, dist_tree))
}

fn inflate_zlib(zlib: &[u8], max_out: usize) -> Result<Vec<u8>, ImageError> {
    if zlib.len() < 6 {
        return Err(ImageError::new("truncated zlib stream"));
    }
    let cmf = zlib[0];
    let flg = zlib[1];
    if !((cmf as u16) * 256 + (flg as u16)).is_multiple_of(31) {
        return Err(ImageError::new("invalid zlib header checksum"));
    }
    if (cmf & 0x0F) != 8 || (cmf >> 4) > 7 {
        return Err(ImageError::new("unsupported zlib compression parameters"));
    }
    if (flg & 0x20) != 0 {
        return Err(ImageError::new("unsupported zlib preset dictionary"));
    }

    let mut reader = BitReader::new(&zlib[2..]);
    let mut out = Vec::with_capacity(max_out.min(1 << 20));

    loop {
        let bfinal = reader.read_bits(1)?;
        let btype = reader.read_bits(2)?;
        match btype {
            0 => {
                let len = reader.read_aligned_u16_le()?;
                let nlen = reader.read_aligned_u16_le()?;
                if len != !nlen {
                    return Err(ImageError::new("corrupt deflate stored block length"));
                }
                let block_len = len as usize;
                if out.len().saturating_add(block_len) > max_out {
                    return Err(ImageError::new("decompressed PNG data exceeds expected size"));
                }
                reader.read_aligned_bytes(block_len, &mut out)?;
            }
            1 | 2 => {
                let (lit_tree, dist_tree) = if btype == 1 {
                    fixed_huffman_trees()?
                } else {
                    decode_dynamic_trees(&mut reader)?
                };
                inflate_huffman_block(&mut reader, &lit_tree, &dist_tree, max_out, &mut out)?;
            }
            _ => return Err(ImageError::new("reserved deflate block type BTYPE=3")),
        }
        if bfinal == 1 {
            break;
        }
    }

    let end_pos = 2 + reader.finish_aligned_byte_pos();
    if end_pos + 4 > zlib.len() {
        return Err(ImageError::new("missing zlib Adler-32 trailer"));
    }
    let expected_adler = u32::from_be_bytes([
        zlib[end_pos],
        zlib[end_pos + 1],
        zlib[end_pos + 2],
        zlib[end_pos + 3],
    ]);
    let actual_adler = adler32(&out);
    if actual_adler != expected_adler {
        return Err(ImageError::new("zlib Adler-32 checksum mismatch"));
    }

    Ok(out)
}

fn inflate_huffman_block(
    reader: &mut BitReader<'_>,
    lit_tree: &HuffmanTree,
    dist_tree: &HuffmanTree,
    max_out: usize,
    out: &mut Vec<u8>,
) -> Result<(), ImageError> {
    const LEN_BASE: [u16; 29] = [
        3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115,
        131, 163, 195, 227, 258,
    ];
    const LEN_EXTRA: [u8; 29] = [
        0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
    ];
    const DIST_BASE: [u16; 30] = [
        1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
        2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
    ];
    const DIST_EXTRA: [u8; 30] = [
        0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12,
        13, 13,
    ];

    loop {
        let sym = lit_tree.decode(reader)?;
        if sym < 256 {
            if out.len() >= max_out {
                return Err(ImageError::new("decompressed PNG data exceeds expected size"));
            }
            out.push(sym as u8);
        } else if sym == 256 {
            return Ok(());
        } else {
            let len_idx = (sym - 257) as usize;
            if len_idx >= LEN_BASE.len() {
                return Err(ImageError::new("invalid deflate length symbol"));
            }
            let length =
                LEN_BASE[len_idx] as usize + reader.read_bits(LEN_EXTRA[len_idx])? as usize;

            let dist_sym = dist_tree.decode(reader)? as usize;
            if dist_sym >= DIST_BASE.len() {
                return Err(ImageError::new("invalid deflate distance symbol"));
            }
            let dist =
                DIST_BASE[dist_sym] as usize + reader.read_bits(DIST_EXTRA[dist_sym])? as usize;
            if dist == 0 || dist > out.len() {
                return Err(ImageError::new("deflate back-reference out of bounds"));
            }
            if out.len().saturating_add(length) > max_out {
                return Err(ImageError::new("decompressed PNG data exceeds expected size"));
            }
            for _ in 0..length {
                let b = out[out.len() - dist];
                out.push(b);
            }
        }
    }
}

// ============================================================================
// BMP Decoder (24-bit and 32-bit BI_RGB / BI_BITFIELDS)
// ============================================================================

fn decode_bmp(bytes: &[u8]) -> Result<DecodedImage, ImageError> {
    if bytes.len() < 54 || &bytes[0..2] != b"BM" {
        return Err(ImageError::new("invalid or truncated BMP header"));
    }

    let pixel_offset = u32::from_le_bytes([bytes[10], bytes[11], bytes[12], bytes[13]]) as usize;
    let dib_size = u32::from_le_bytes([bytes[14], bytes[15], bytes[16], bytes[17]]);
    if dib_size < 40 {
        return Err(ImageError::new(format!(
            "unsupported BMP DIB header size {}",
            dib_size
        )));
    }
    if bytes.len() < 14 + dib_size as usize {
        return Err(ImageError::new("truncated BMP DIB header"));
    }

    let width_i32 = i32::from_le_bytes([bytes[18], bytes[19], bytes[20], bytes[21]]);
    let height_i32 = i32::from_le_bytes([bytes[22], bytes[23], bytes[24], bytes[25]]);
    if width_i32 <= 0 || height_i32 == 0 || height_i32 == i32::MIN {
        return Err(ImageError::new("invalid BMP dimensions"));
    }
    let width = width_i32 as u32;
    let top_down = height_i32 < 0;
    let height = height_i32.unsigned_abs();
    let rgba_len = validate_dimensions(width, height)?;

    let planes = u16::from_le_bytes([bytes[26], bytes[27]]);
    if planes != 1 {
        return Err(ImageError::new("invalid BMP plane count"));
    }
    let bpp = u16::from_le_bytes([bytes[28], bytes[29]]);
    if bpp != 24 && bpp != 32 {
        return Err(ImageError::new(format!("unsupported BMP bit depth {}", bpp)));
    }
    let compression = u32::from_le_bytes([bytes[30], bytes[31], bytes[32], bytes[33]]);
    if !matches!(compression, 0 | 3 | 6) {
        return Err(ImageError::new(format!(
            "unsupported BMP compression {}",
            compression
        )));
    }
    if bpp == 24 && compression != 0 {
        return Err(ImageError::new("24-bit BMP only supports BI_RGB"));
    }

    let (r_mask, g_mask, b_mask, a_mask) = if compression == 3 || compression == 6 {
        if bytes.len() < 66 {
            return Err(ImageError::new("truncated BMP BI_BITFIELDS masks"));
        }
        let r = u32::from_le_bytes([bytes[54], bytes[55], bytes[56], bytes[57]]);
        let g = u32::from_le_bytes([bytes[58], bytes[59], bytes[60], bytes[61]]);
        let b = u32::from_le_bytes([bytes[62], bytes[63], bytes[64], bytes[65]]);
        let a = if (dib_size >= 56 || compression == 6) && bytes.len() >= 70 {
            u32::from_le_bytes([bytes[66], bytes[67], bytes[68], bytes[69]])
        } else {
            0
        };
        (r, g, b, a)
    } else {
        (0, 0, 0, 0)
    };

    let bytes_per_pixel = (bpp / 8) as usize;
    let row_bytes = (width as usize)
        .checked_mul(bytes_per_pixel)
        .ok_or_else(|| ImageError::new("BMP stride overflow"))?;
    let stride = (row_bytes + 3) & !3;
    let needed = (height as usize)
        .checked_mul(stride)
        .and_then(|v| pixel_offset.checked_add(v))
        .ok_or_else(|| ImageError::new("BMP pixel array size overflow"))?;
    if needed > bytes.len() {
        return Err(ImageError::new("truncated BMP pixel data"));
    }

    let mut rgba = vec![0u8; rgba_len];
    let w = width as usize;
    let h = height as usize;

    for row in 0..h {
        let src_row = if top_down { row } else { h - 1 - row };
        let src_off = pixel_offset + src_row * stride;
        let dst_off = row * w * 4;
        let src = &bytes[src_off..src_off + row_bytes];
        let dst = &mut rgba[dst_off..dst_off + w * 4];

        if bpp == 24 {
            for x in 0..w {
                let s = x * 3;
                let d = x * 4;
                dst[d] = src[s + 2];
                dst[d + 1] = src[s + 1];
                dst[d + 2] = src[s];
                dst[d + 3] = 255;
            }
        } else if compression == 0 {
            for x in 0..w {
                let s = x * 4;
                let d = x * 4;
                dst[d] = src[s + 2];
                dst[d + 1] = src[s + 1];
                dst[d + 2] = src[s];
                dst[d + 3] = src[s + 3];
            }
        } else {
            for x in 0..w {
                let s = x * 4;
                let d = x * 4;
                let word = u32::from_le_bytes([src[s], src[s + 1], src[s + 2], src[s + 3]]);
                dst[d] = extract_bitfield(word, r_mask, 0);
                dst[d + 1] = extract_bitfield(word, g_mask, 0);
                dst[d + 2] = extract_bitfield(word, b_mask, 0);
                dst[d + 3] = extract_bitfield(word, a_mask, 255);
            }
        }
    }

    Ok(DecodedImage {
        width,
        height,
        rgba,
    })
}

fn extract_bitfield(word: u32, mask: u32, default_if_zero: u8) -> u8 {
    if mask == 0 {
        return default_if_zero;
    }
    let shift = mask.trailing_zeros();
    let shifted_mask = mask >> shift;
    let bits = shifted_mask.trailing_ones();
    if bits == 0 {
        return default_if_zero;
    }
    let val = (word & mask) >> shift;
    if bits == 8 {
        val as u8
    } else if bits > 8 {
        (val >> (bits - 8)) as u8
    } else {
        let max_val = (1u32 << bits) - 1;
        ((val * 255) / max_val) as u8
    }
}

// ============================================================================
// PPM Decoder (P6 binary & P3 ASCII)
// ============================================================================

fn decode_ppm(bytes: &[u8]) -> Result<DecodedImage, ImageError> {
    let is_binary = if bytes.starts_with(b"P6") {
        true
    } else if bytes.starts_with(b"P3") {
        false
    } else {
        return Err(ImageError::new("invalid PPM magic"));
    };

    let mut pos = 2usize;
    if bytes.get(pos).is_none_or(|b| !b.is_ascii_whitespace()) {
        return Err(ImageError::new("invalid PPM header whitespace"));
    }

    let width = read_ppm_u32(bytes, &mut pos)?;
    let height = read_ppm_u32(bytes, &mut pos)?;
    let rgba_len = validate_dimensions(width, height)?;
    let maxval = read_ppm_u32(bytes, &mut pos)?;
    if maxval == 0 || maxval > 65535 {
        return Err(ImageError::new("invalid PPM maxval"));
    }

    let mut rgba = vec![0u8; rgba_len];
    let pixel_count = (width as usize) * (height as usize);

    if is_binary {
        // P6: exactly one whitespace byte separates maxval from raw RGB samples.
        if bytes.get(pos).is_none_or(|b| !b.is_ascii_whitespace()) {
            return Err(ImageError::new("missing separator after PPM P6 maxval"));
        }
        pos += 1;
        let sample_bytes = if maxval > 255 { 2 } else { 1 };
        let needed = pixel_count
            .checked_mul(3 * sample_bytes)
            .and_then(|v| pos.checked_add(v))
            .ok_or_else(|| ImageError::new("PPM P6 payload overflow"))?;
        if needed > bytes.len() {
            return Err(ImageError::new("truncated PPM P6 pixel data"));
        }

        if sample_bytes == 1 {
            for i in 0..pixel_count {
                let s = pos + i * 3;
                let d = i * 4;
                rgba[d] = scale_ppm(bytes[s] as u32, maxval);
                rgba[d + 1] = scale_ppm(bytes[s + 1] as u32, maxval);
                rgba[d + 2] = scale_ppm(bytes[s + 2] as u32, maxval);
                rgba[d + 3] = 255;
            }
        } else {
            for i in 0..pixel_count {
                let s = pos + i * 6;
                let d = i * 4;
                let r = u16::from_be_bytes([bytes[s], bytes[s + 1]]) as u32;
                let g = u16::from_be_bytes([bytes[s + 2], bytes[s + 3]]) as u32;
                let b = u16::from_be_bytes([bytes[s + 4], bytes[s + 5]]) as u32;
                rgba[d] = scale_ppm(r, maxval);
                rgba[d + 1] = scale_ppm(g, maxval);
                rgba[d + 2] = scale_ppm(b, maxval);
                rgba[d + 3] = 255;
            }
        }
    } else {
        for i in 0..pixel_count {
            let r = read_ppm_u32(bytes, &mut pos)?;
            let g = read_ppm_u32(bytes, &mut pos)?;
            let b = read_ppm_u32(bytes, &mut pos)?;
            if r > maxval || g > maxval || b > maxval {
                return Err(ImageError::new("PPM P3 sample exceeds maxval"));
            }
            let d = i * 4;
            rgba[d] = scale_ppm(r, maxval);
            rgba[d + 1] = scale_ppm(g, maxval);
            rgba[d + 2] = scale_ppm(b, maxval);
            rgba[d + 3] = 255;
        }
    }

    Ok(DecodedImage {
        width,
        height,
        rgba,
    })
}

fn scale_ppm(sample: u32, maxval: u32) -> u8 {
    if maxval == 255 {
        sample as u8
    } else {
        ((sample.min(maxval) * 255) / maxval) as u8
    }
}

fn read_ppm_u32(bytes: &[u8], pos: &mut usize) -> Result<u32, ImageError> {
    while *pos < bytes.len() {
        let b = bytes[*pos];
        if b.is_ascii_whitespace() {
            *pos += 1;
        } else if b == b'#' {
            while *pos < bytes.len() && bytes[*pos] != b'\n' && bytes[*pos] != b'\r' {
                *pos += 1;
            }
        } else {
            break;
        }
    }
    let start = *pos;
    let mut val: u32 = 0;
    while *pos < bytes.len() && bytes[*pos].is_ascii_digit() {
        let digit = (bytes[*pos] - b'0') as u32;
        val = val
            .checked_mul(10)
            .and_then(|v| v.checked_add(digit))
            .ok_or_else(|| ImageError::new("PPM integer overflow"))?;
        *pos += 1;
    }
    if *pos == start {
        return Err(ImageError::new("expected integer in PPM"));
    }
    Ok(val)
}

// ============================================================================
// Basic Static SVG Rasterizer (<svg>, <g>, <rect>, <circle>, <line>)
// ============================================================================

#[derive(Debug, Clone)]
struct SvgPaintState {
    fill: Option<[u8; 4]>,
    stroke: Option<[u8; 4]>,
    stroke_width: f32,
    opacity: f32,
    fill_opacity: f32,
    stroke_opacity: f32,
}

impl Default for SvgPaintState {
    fn default() -> Self {
        Self {
            fill: Some([0, 0, 0, 255]),
            stroke: None,
            stroke_width: 1.0,
            opacity: 1.0,
            fill_opacity: 1.0,
            stroke_opacity: 1.0,
        }
    }
}

impl SvgPaintState {
    fn apply_attrs(&mut self, attrs: &[(String, String)]) {
        if let Some(style) = find_attr(attrs, "style") {
            for decl in style.split(';') {
                if let Some((k, v)) = decl.split_once(':') {
                    self.apply_prop(k.trim(), v.trim());
                }
            }
        }
        for (k, v) in attrs {
            if !k.eq_ignore_ascii_case("style") {
                self.apply_prop(k, v);
            }
        }
    }

    fn apply_prop(&mut self, key: &str, val: &str) {
        if key.eq_ignore_ascii_case("fill") {
            if let Some(c) = parse_svg_paint(val) {
                self.fill = c;
            }
        } else if key.eq_ignore_ascii_case("stroke") {
            if let Some(c) = parse_svg_paint(val) {
                self.stroke = c;
            }
        } else if key.eq_ignore_ascii_case("stroke-width") {
            if let Some(w) = parse_svg_length(val) {
                self.stroke_width = w.max(0.0);
            }
        } else if key.eq_ignore_ascii_case("opacity") {
            if let Ok(a) = val.trim().parse::<f32>() {
                self.opacity = a.clamp(0.0, 1.0);
            }
        } else if key.eq_ignore_ascii_case("fill-opacity") {
            if let Ok(a) = val.trim().parse::<f32>() {
                self.fill_opacity = a.clamp(0.0, 1.0);
            }
        } else if key.eq_ignore_ascii_case("stroke-opacity") {
            if let Ok(a) = val.trim().parse::<f32>() {
                self.stroke_opacity = a.clamp(0.0, 1.0);
            }
        }
    }

    fn effective_fill(&self) -> Option<[u8; 4]> {
        self.fill.map(|mut c| {
            c[3] = ((c[3] as f32) * self.opacity * self.fill_opacity)
                .round()
                .clamp(0.0, 255.0) as u8;
            c
        })
    }

    fn effective_stroke(&self) -> Option<[u8; 4]> {
        if self.stroke_width <= 0.0 {
            return None;
        }
        self.stroke.map(|mut c| {
            c[3] = ((c[3] as f32) * self.opacity * self.stroke_opacity)
                .round()
                .clamp(0.0, 255.0) as u8;
            c
        })
    }
}

fn decode_svg(bytes: &[u8]) -> Result<DecodedImage, ImageError> {
    let trimmed = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let text = std::str::from_utf8(trimmed)
        .map_err(|_| ImageError::new("SVG payload is not valid UTF-8"))?;
    let mut cursor = skip_xml_preamble(text);
    if !starts_with_ascii_ci(cursor, "<svg") {
        return Err(ImageError::new("expected <svg> root element"));
    }

    let (svg_tag, after_svg, root_self_closing) = parse_open_tag(cursor)?;
    if !svg_tag.name.eq_ignore_ascii_case("svg") {
        return Err(ImageError::new("expected <svg> root element"));
    }

    let vb = find_attr(&svg_tag.attrs, "viewBox")
        .and_then(parse_viewbox)
        .filter(|&(_, _, vw, vh)| vw > 0.0 && vh > 0.0);
    let parsed_w = find_attr(&svg_tag.attrs, "width").and_then(parse_svg_dim);
    let parsed_h = find_attr(&svg_tag.attrs, "height").and_then(parse_svg_dim);

    let (width_f, height_f) = match (parsed_w, parsed_h, vb) {
        (Some(w), Some(h), _) => (w, h),
        (Some(w), None, Some((_, _, vw, vh))) => (w, w * vh / vw),
        (None, Some(h), Some((_, _, vw, vh))) => (h * vw / vh, h),
        (None, None, Some((_, _, vw, vh))) => (vw, vh),
        (Some(w), None, None) => (w, 128.0),
        (None, Some(h), None) => (128.0, h),
        (None, None, None) => (128.0, 128.0),
    };

    if !width_f.is_finite() || !height_f.is_finite() || width_f <= 0.0 || height_f <= 0.0 {
        return Err(ImageError::new("invalid SVG dimensions"));
    }
    if width_f > (MAX_IMAGE_PIXELS as f32) || height_f > (MAX_IMAGE_PIXELS as f32) {
        return Err(ImageError::new("SVG dimensions exceed maximum pixel limit"));
    }
    let width = width_f.round().max(1.0) as u32;
    let height = height_f.round().max(1.0) as u32;
    let rgba_len = validate_dimensions(width, height)?;
    let mut rgba = vec![0u8; rgba_len];

    if root_self_closing {
        return Ok(DecodedImage {
            width,
            height,
            rgba,
        });
    }

    let (vb_x, vb_y, vb_w, vb_h) = match vb {
        Some((vx, vy, vw, vh)) if vw > 0.0 && vh > 0.0 => (vx, vy, vw, vh),
        _ => (0.0, 0.0, width as f32, height as f32),
    };
    let sx = width as f32 / vb_w;
    let sy = height as f32 / vb_h;
    let uniform_scale = (sx + sy) * 0.5;

    let mut root_state = SvgPaintState::default();
    root_state.apply_attrs(&svg_tag.attrs);
    let mut stack = vec![root_state];

    cursor = after_svg;
    while !cursor.is_empty() {
        let Some(lt) = cursor.find('<') else {
            break;
        };
        cursor = &cursor[lt..];
        if cursor.starts_with("<!--") {
            if let Some(end) = cursor.find("-->") {
                cursor = &cursor[end + 3..];
                continue;
            }
            break;
        }
        if cursor.starts_with("</") {
            let Some(gt) = cursor.find('>') else {
                break;
            };
            let close_name = cursor[2..gt].trim();
            if close_name.eq_ignore_ascii_case("svg") {
                break;
            }
            if close_name.eq_ignore_ascii_case("g") && stack.len() > 1 {
                stack.pop();
            }
            cursor = &cursor[gt + 1..];
            continue;
        }
        if cursor.starts_with("<?") || cursor.starts_with("<!") {
            if let Some(gt) = cursor.find('>') {
                cursor = &cursor[gt + 1..];
                continue;
            }
            break;
        }

        let (tag, next_cursor, self_closing) = parse_open_tag(cursor)?;
        cursor = next_cursor;

        if tag.name.eq_ignore_ascii_case("defs") || tag.name.eq_ignore_ascii_case("style") {
            if !self_closing {
                let close = if tag.name.eq_ignore_ascii_case("defs") {
                    "</defs>"
                } else {
                    "</style>"
                };
                if let Some(end) = find_ascii_ci(cursor, close) {
                    cursor = &cursor[end + close.len()..];
                }
            }
            continue;
        }

        let mut state = stack.last().cloned().unwrap_or_default();
        state.apply_attrs(&tag.attrs);

        if tag.name.eq_ignore_ascii_case("g") {
            if !self_closing {
                stack.push(state);
            }
            continue;
        }

        if tag.name.eq_ignore_ascii_case("rect") {
            let x = (attr_f32(&tag.attrs, "x", 0.0) - vb_x) * sx;
            let y = (attr_f32(&tag.attrs, "y", 0.0) - vb_y) * sy;
            let w = attr_f32(&tag.attrs, "width", 0.0) * sx;
            let h = attr_f32(&tag.attrs, "height", 0.0) * sy;
            if w > 0.0 && h > 0.0 {
                if let Some(fill) = state.effective_fill() {
                    raster_rect_fill(&mut rgba, width, height, [x, y, w, h], fill);
                }
                if let Some(stroke) = state.effective_stroke() {
                    let sw = state.stroke_width * uniform_scale;
                    raster_rect_stroke(&mut rgba, width, height, x, y, w, h, sw, stroke);
                }
            }
        } else if tag.name.eq_ignore_ascii_case("circle") {
            let cx = (attr_f32(&tag.attrs, "cx", 0.0) - vb_x) * sx;
            let cy = (attr_f32(&tag.attrs, "cy", 0.0) - vb_y) * sy;
            let r = attr_f32(&tag.attrs, "r", 0.0) * uniform_scale;
            if r > 0.0 {
                if let Some(fill) = state.effective_fill() {
                    raster_circle_fill(&mut rgba, width, height, cx, cy, r, fill);
                }
                if let Some(stroke) = state.effective_stroke() {
                    let sw = state.stroke_width * uniform_scale;
                    raster_circle_stroke(&mut rgba, width, height, cx, cy, r, sw, stroke);
                }
            }
        } else if tag.name.eq_ignore_ascii_case("line") {
            let x1 = (attr_f32(&tag.attrs, "x1", 0.0) - vb_x) * sx;
            let y1 = (attr_f32(&tag.attrs, "y1", 0.0) - vb_y) * sy;
            let x2 = (attr_f32(&tag.attrs, "x2", 0.0) - vb_x) * sx;
            let y2 = (attr_f32(&tag.attrs, "y2", 0.0) - vb_y) * sy;
            if let Some(stroke) = state.effective_stroke() {
                let sw = state.stroke_width * uniform_scale;
                raster_line(&mut rgba, width, height, x1, y1, x2, y2, sw, stroke);
            }
        }
    }

    Ok(DecodedImage {
        width,
        height,
        rgba,
    })
}

struct XmlTag {
    name: String,
    attrs: Vec<(String, String)>,
}

fn parse_open_tag(s: &str) -> Result<(XmlTag, &str, bool), ImageError> {
    if !s.starts_with('<') {
        return Err(ImageError::new("malformed SVG tag"));
    }
    let bytes = s.as_bytes();
    let mut i = 1usize;
    let name_start = i;
    while i < bytes.len()
        && !bytes[i].is_ascii_whitespace()
        && bytes[i] != b'>'
        && bytes[i] != b'/'
    {
        i += 1;
    }
    let name = s[name_start..i].to_string();
    if name.is_empty() {
        return Err(ImageError::new("empty SVG tag name"));
    }

    let mut attrs = Vec::new();
    let mut self_closing = false;

    loop {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            return Err(ImageError::new("unterminated SVG tag"));
        }
        if bytes[i] == b'>' {
            i += 1;
            break;
        }
        if bytes[i] == b'/' {
            self_closing = true;
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i >= bytes.len() || bytes[i] != b'>' {
                return Err(ImageError::new("malformed self-closing SVG tag"));
            }
            i += 1;
            break;
        }

        let k_start = i;
        while i < bytes.len()
            && !bytes[i].is_ascii_whitespace()
            && bytes[i] != b'='
            && bytes[i] != b'>'
            && bytes[i] != b'/'
        {
            i += 1;
        }
        let key = s[k_start..i].to_string();
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut val = String::new();
        if i < bytes.len() && bytes[i] == b'=' {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < bytes.len() && (bytes[i] == b'"' || bytes[i] == b'\'') {
                let quote = bytes[i];
                i += 1;
                let v_start = i;
                while i < bytes.len() && bytes[i] != quote {
                    i += 1;
                }
                if i >= bytes.len() {
                    return Err(ImageError::new("unterminated SVG attribute string"));
                }
                val = s[v_start..i].to_string();
                i += 1;
            } else {
                let v_start = i;
                while i < bytes.len()
                    && !bytes[i].is_ascii_whitespace()
                    && bytes[i] != b'>'
                    && bytes[i] != b'/'
                {
                    i += 1;
                }
                val = s[v_start..i].to_string();
            }
        }
        if !key.is_empty() {
            attrs.push((key, val));
        }
    }

    Ok((XmlTag { name, attrs }, &s[i..], self_closing))
}

fn find_attr<'a>(attrs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

fn attr_f32(attrs: &[(String, String)], name: &str, default: f32) -> f32 {
    find_attr(attrs, name)
        .and_then(parse_svg_length)
        .unwrap_or(default)
}

fn parse_svg_length(s: &str) -> Option<f32> {
    let t = s.trim();
    let t = t.strip_suffix("px").unwrap_or(t).trim();
    t.parse::<f32>().ok().filter(|v| v.is_finite())
}

fn parse_svg_dim(s: &str) -> Option<f32> {
    let t = s.trim();
    if t.ends_with('%') {
        return None;
    }
    parse_svg_length(t)
}

fn parse_viewbox(s: &str) -> Option<(f32, f32, f32, f32)> {
    let nums: Vec<f32> = s
        .split(|c: char| c.is_ascii_whitespace() || c == ',')
        .filter(|p| !p.is_empty())
        .filter_map(|p| p.parse::<f32>().ok())
        .collect();
    if nums.len() == 4 && nums.iter().all(|v| v.is_finite()) {
        Some((nums[0], nums[1], nums[2], nums[3]))
    } else {
        None
    }
}

fn parse_svg_paint(s: &str) -> Option<Option<[u8; 4]>> {
    let t = s.trim();
    if t.eq_ignore_ascii_case("none") {
        return Some(None);
    }
    if t.eq_ignore_ascii_case("transparent") {
        return Some(Some([0, 0, 0, 0]));
    }
    if let Some(hex) = t.strip_prefix('#') {
        return parse_hex_color(hex).map(Some);
    }
    if starts_with_ascii_ci(t, "rgb(") || starts_with_ascii_ci(t, "rgba(") {
        return parse_rgb_func(t).map(Some);
    }
    named_svg_color(t).map(|rgb| Some([rgb[0], rgb[1], rgb[2], 255]))
}

fn parse_hex_color(hex: &str) -> Option<[u8; 4]> {
    let b = hex.as_bytes();
    let nibble = |c: u8| -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        }
    };
    let byte2 = |hi: u8, lo: u8| -> Option<u8> { Some((nibble(hi)? << 4) | nibble(lo)?) };
    match b.len() {
        3 => {
            let r = nibble(b[0])? * 17;
            let g = nibble(b[1])? * 17;
            let b_ch = nibble(b[2])? * 17;
            Some([r, g, b_ch, 255])
        }
        4 => {
            let r = nibble(b[0])? * 17;
            let g = nibble(b[1])? * 17;
            let b_ch = nibble(b[2])? * 17;
            let a = nibble(b[3])? * 17;
            Some([r, g, b_ch, a])
        }
        6 => Some([
            byte2(b[0], b[1])?,
            byte2(b[2], b[3])?,
            byte2(b[4], b[5])?,
            255,
        ]),
        8 => Some([
            byte2(b[0], b[1])?,
            byte2(b[2], b[3])?,
            byte2(b[4], b[5])?,
            byte2(b[6], b[7])?,
        ]),
        _ => None,
    }
}

fn parse_rgb_func(s: &str) -> Option<[u8; 4]> {
    let inner = s.split_once('(')?.1.strip_suffix(')')?;
    let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
    if parts.len() != 3 && parts.len() != 4 {
        return None;
    }
    let r = parts[0].parse::<f32>().ok()?.round().clamp(0.0, 255.0) as u8;
    let g = parts[1].parse::<f32>().ok()?.round().clamp(0.0, 255.0) as u8;
    let b = parts[2].parse::<f32>().ok()?.round().clamp(0.0, 255.0) as u8;
    let a = if parts.len() == 4 {
        (parts[3].parse::<f32>().ok()?.clamp(0.0, 1.0) * 255.0).round() as u8
    } else {
        255
    };
    Some([r, g, b, a])
}

fn named_svg_color(name: &str) -> Option<[u8; 3]> {
    let lower = name.to_ascii_lowercase();
    let rgb = match lower.as_str() {
        "black" | "currentcolor" => [0, 0, 0],
        "white" => [255, 255, 255],
        "red" => [255, 0, 0],
        "green" => [0, 128, 0],
        "lime" => [0, 255, 0],
        "blue" => [0, 0, 255],
        "yellow" => [255, 255, 0],
        "cyan" | "aqua" => [0, 255, 255],
        "magenta" | "fuchsia" => [255, 0, 255],
        "gray" | "grey" => [128, 128, 128],
        "silver" => [192, 192, 192],
        "lightgray" | "lightgrey" => [211, 211, 211],
        "darkgray" | "darkgrey" => [169, 169, 169],
        "maroon" => [128, 0, 0],
        "darkred" => [139, 0, 0],
        "navy" => [0, 0, 128],
        "darkblue" => [0, 0, 139],
        "darkgreen" => [0, 100, 0],
        "olive" => [128, 128, 0],
        "purple" => [128, 0, 128],
        "rebeccapurple" => [102, 51, 153],
        "teal" => [0, 128, 128],
        "orange" => [255, 165, 0],
        "pink" => [255, 192, 203],
        "brown" => [165, 42, 42],
        "gold" => [255, 215, 0],
        "indigo" => [75, 0, 130],
        "violet" => [238, 130, 238],
        "coral" => [255, 127, 80],
        "crimson" => [220, 20, 60],
        "lightblue" => [173, 216, 230],
        "skyblue" => [135, 206, 235],
        _ => return None,
    };
    Some(rgb)
}

fn blend_rgba(dst: &mut [u8], src: [u8; 4], coverage: f32) {
    let sa = (src[3] as f32 / 255.0) * coverage.clamp(0.0, 1.0);
    if sa <= 0.0 {
        return;
    }
    if sa >= 0.9999 {
        dst[0] = src[0];
        dst[1] = src[1];
        dst[2] = src[2];
        dst[3] = 255;
        return;
    }
    let da = dst[3] as f32 / 255.0;
    let out_a = sa + da * (1.0 - sa);
    if out_a <= 0.0 {
        return;
    }
    let inv_sa = 1.0 - sa;
    for c in 0..3 {
        let out_c = (src[c] as f32 * sa + dst[c] as f32 * da * inv_sa) / out_a;
        dst[c] = out_c.round().clamp(0.0, 255.0) as u8;
    }
    dst[3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
}

fn raster_rect_fill(
    rgba: &mut [u8],
    img_w: u32,
    img_h: u32,
    [x, y, w, h]: [f32; 4],
    color: [u8; 4],
) {
    let x0 = (x - 1.0).floor().max(0.0) as u32;
    let y0 = (y - 1.0).floor().max(0.0) as u32;
    let x1 = ((x + w + 1.0).ceil().max(0.0) as u32).min(img_w);
    let y1 = ((y + h + 1.0).ceil().max(0.0) as u32).min(img_h);
    for py in y0..y1 {
        let cy = py as f32 + 0.5;
        let cy_cov = (cy - y + 0.5).min(y + h - cy + 0.5).clamp(0.0, 1.0);
        if cy_cov <= 0.0 {
            continue;
        }
        for px in x0..x1 {
            let cx = px as f32 + 0.5;
            let cx_cov = (cx - x + 0.5).min(x + w - cx + 0.5).clamp(0.0, 1.0);
            let cov = cx_cov * cy_cov;
            if cov > 0.0 {
                let idx = ((py * img_w + px) * 4) as usize;
                blend_rgba(&mut rgba[idx..idx + 4], color, cov);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn raster_rect_stroke(
    rgba: &mut [u8],
    img_w: u32,
    img_h: u32,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    stroke_w: f32,
    color: [u8; 4],
) {
    let half = stroke_w * 0.5;
    let cx_box = x + w * 0.5;
    let cy_box = y + h * 0.5;
    let hw = w * 0.5;
    let hh = h * 0.5;

    let x0 = (x - half - 1.0).floor().max(0.0) as u32;
    let y0 = (y - half - 1.0).floor().max(0.0) as u32;
    let x1 = ((x + w + half + 1.0).ceil().max(0.0) as u32).min(img_w);
    let y1 = ((y + h + half + 1.0).ceil().max(0.0) as u32).min(img_h);

    for py in y0..y1 {
        let dy = (py as f32 + 0.5 - cy_box).abs() - hh;
        for px in x0..x1 {
            let dx = (px as f32 + 0.5 - cx_box).abs() - hw;
            let outside = dx.max(0.0).hypot(dy.max(0.0));
            let inside = dx.max(dy).min(0.0);
            let sd_rect = outside + inside;
            let cov = (half - sd_rect.abs() + 0.5).clamp(0.0, 1.0);
            if cov > 0.0 {
                let idx = ((py * img_w + px) * 4) as usize;
                blend_rgba(&mut rgba[idx..idx + 4], color, cov);
            }
        }
    }
}

fn raster_circle_fill(
    rgba: &mut [u8],
    img_w: u32,
    img_h: u32,
    cx: f32,
    cy: f32,
    r: f32,
    color: [u8; 4],
) {
    let x0 = (cx - r - 1.0).floor().max(0.0) as u32;
    let y0 = (cy - r - 1.0).floor().max(0.0) as u32;
    let x1 = ((cx + r + 1.0).ceil().max(0.0) as u32).min(img_w);
    let y1 = ((cy + r + 1.0).ceil().max(0.0) as u32).min(img_h);

    for py in y0..y1 {
        let dy = py as f32 + 0.5 - cy;
        for px in x0..x1 {
            let dx = px as f32 + 0.5 - cx;
            let dist = dx.hypot(dy);
            let cov = (r - dist + 0.5).clamp(0.0, 1.0);
            if cov > 0.0 {
                let idx = ((py * img_w + px) * 4) as usize;
                blend_rgba(&mut rgba[idx..idx + 4], color, cov);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn raster_circle_stroke(
    rgba: &mut [u8],
    img_w: u32,
    img_h: u32,
    cx: f32,
    cy: f32,
    r: f32,
    stroke_w: f32,
    color: [u8; 4],
) {
    let half = stroke_w * 0.5;
    let outer = r + half + 1.0;
    let x0 = (cx - outer).floor().max(0.0) as u32;
    let y0 = (cy - outer).floor().max(0.0) as u32;
    let x1 = ((cx + outer).ceil().max(0.0) as u32).min(img_w);
    let y1 = ((cy + outer).ceil().max(0.0) as u32).min(img_h);

    for py in y0..y1 {
        let dy = py as f32 + 0.5 - cy;
        for px in x0..x1 {
            let dx = px as f32 + 0.5 - cx;
            let dist = (dx.hypot(dy) - r).abs();
            let cov = (half - dist + 0.5).clamp(0.0, 1.0);
            if cov > 0.0 {
                let idx = ((py * img_w + px) * 4) as usize;
                blend_rgba(&mut rgba[idx..idx + 4], color, cov);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn raster_line(
    rgba: &mut [u8],
    img_w: u32,
    img_h: u32,
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    stroke_w: f32,
    color: [u8; 4],
) {
    let half = stroke_w * 0.5;
    let pad = half + 1.0;
    let min_x = (x1.min(x2) - pad).floor().max(0.0) as u32;
    let min_y = (y1.min(y2) - pad).floor().max(0.0) as u32;
    let max_x = ((x1.max(x2) + pad).ceil().max(0.0) as u32).min(img_w);
    let max_y = ((y1.max(y2) + pad).ceil().max(0.0) as u32).min(img_h);

    let vx = x2 - x1;
    let vy = y2 - y1;
    let len_sq = vx * vx + vy * vy;

    for py in min_y..max_y {
        let cy = py as f32 + 0.5;
        for px in min_x..max_x {
            let cx = px as f32 + 0.5;
            let t = if len_sq > 1e-6 {
                (((cx - x1) * vx + (cy - y1) * vy) / len_sq).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let proj_x = x1 + t * vx;
            let proj_y = y1 + t * vy;
            let dist = (cx - proj_x).hypot(cy - proj_y);
            let cov = (half - dist + 0.5).clamp(0.0, 1.0);
            if cov > 0.0 {
                let idx = ((py * img_w + px) * 4) as usize;
                blend_rgba(&mut rgba[idx..idx + 4], color, cov);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push_png_chunk(out: &mut Vec<u8>, chunk_type: &[u8; 4], payload: &[u8]) {
        out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        out.extend_from_slice(chunk_type);
        out.extend_from_slice(payload);
        let c = crc32(&[chunk_type, payload]);
        out.extend_from_slice(&c.to_be_bytes());
    }

    fn zlib_stored(raw: &[u8]) -> Vec<u8> {
        let mut z = vec![0x78, 0x01]; // CMF=8/32K, FLG=1 ((0x7801 % 31) == 0)
        let mut offset = 0usize;
        loop {
            let chunk_len = (raw.len() - offset).min(65535);
            let is_final = offset + chunk_len == raw.len();
            z.push(if is_final { 1 } else { 0 });
            let len16 = chunk_len as u16;
            z.extend_from_slice(&len16.to_le_bytes());
            z.extend_from_slice(&(!len16).to_le_bytes());
            z.extend_from_slice(&raw[offset..offset + chunk_len]);
            offset += chunk_len;
            if is_final {
                break;
            }
        }
        z.extend_from_slice(&adler32(raw).to_be_bytes());
        z
    }

    fn build_png(
        width: u32,
        height: u32,
        bit_depth: u8,
        color_type: u8,
        palette: Option<&[[u8; 3]]>,
        trns: Option<&[u8]>,
        raw_scanlines: &[u8],
    ) -> Vec<u8> {
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut ihdr = Vec::with_capacity(13);
        ihdr.extend_from_slice(&width.to_be_bytes());
        ihdr.extend_from_slice(&height.to_be_bytes());
        ihdr.push(bit_depth);
        ihdr.push(color_type);
        ihdr.push(0); // compression
        ihdr.push(0); // filter method
        ihdr.push(0); // non-interlaced
        push_png_chunk(&mut out, b"IHDR", &ihdr);

        if let Some(pal) = palette {
            let mut p = Vec::with_capacity(pal.len() * 3);
            for rgb in pal {
                p.extend_from_slice(rgb);
            }
            push_png_chunk(&mut out, b"PLTE", &p);
        }
        if let Some(t) = trns {
            push_png_chunk(&mut out, b"tRNS", t);
        }
        let compressed = zlib_stored(raw_scanlines);
        // Split across two IDAT chunks when large enough to verify multi-IDAT assembly.
        let split = compressed.len() / 2;
        if split > 0 {
            push_png_chunk(&mut out, b"IDAT", &compressed[..split]);
            push_png_chunk(&mut out, b"IDAT", &compressed[split..]);
        } else {
            push_png_chunk(&mut out, b"IDAT", &compressed);
        }
        push_png_chunk(&mut out, b"IEND", &[]);
        out
    }

    #[test]
    fn png_rgba_with_sub_up_avg_and_paeth_filters() {
        // 3x4 RGBA image covering all 5 PNG filter types (None, Sub, Up, Average, Paeth)
        // plus alpha transparency.
        let expected: [[[u8; 4]; 3]; 4] = [
            [[255, 0, 0, 255], [0, 255, 0, 128], [0, 0, 255, 0]],
            [[10, 20, 30, 255], [40, 50, 60, 200], [70, 80, 90, 64]],
            [[15, 25, 35, 255], [45, 55, 65, 210], [75, 85, 95, 10]],
            [[200, 150, 100, 255], [100, 150, 200, 180], [50, 60, 70, 0]],
        ];
        let mut raw = Vec::new();
        let row_bytes = 3 * 4;
        let bpp = 4usize;

        for y in 0..4 {
            let cur: Vec<u8> = expected[y].iter().flat_map(|p| *p).collect();
            let prev: Vec<u8> = if y == 0 {
                vec![0u8; row_bytes]
            } else {
                expected[y - 1].iter().flat_map(|p| *p).collect()
            };
            let filter_type = match y {
                0 => 1u8, // Sub
                1 => 2u8, // Up
                2 => 3u8, // Average
                _ => 4u8, // Paeth
            };
            raw.push(filter_type);
            for i in 0..row_bytes {
                let a = if i >= bpp { cur[i - bpp] } else { 0 };
                let b = prev[i];
                let c = if i >= bpp { prev[i - bpp] } else { 0 };
                let encoded = match filter_type {
                    1 => cur[i].wrapping_sub(a),
                    2 => cur[i].wrapping_sub(b),
                    3 => cur[i].wrapping_sub(((a as u16 + b as u16) >> 1) as u8),
                    4 => cur[i].wrapping_sub(paeth_predictor(a, b, c)),
                    _ => cur[i],
                };
                raw.push(encoded);
            }
        }

        let png = build_png(3, 4, 8, 6, None, None, &raw);
        let decoded = decode(&png).expect("valid PNG must decode");
        assert_eq!(decoded.width, 3);
        assert_eq!(decoded.height, 4);
        let flat_expected: Vec<u8> = expected
            .iter()
            .flat_map(|row| row.iter().flat_map(|p| *p))
            .collect();
        assert_eq!(decoded.rgba, flat_expected);
    }

    #[test]
    fn png_indexed_plte_and_trns() {
        let palette = [[10, 20, 30], [200, 100, 50], [255, 255, 255]];
        let trns = [0u8, 128u8]; // index 2 defaults to 255
        let raw = vec![
            0, 0, 1, 2, // row 0: filter=None, indices 0, 1, 2
            0, 2, 1, 0, // row 1: filter=None, indices 2, 1, 0
        ];
        let png = build_png(3, 2, 8, 3, Some(&palette), Some(&trns), &raw);
        let decoded = decode(&png).expect("indexed PNG must decode");
        assert_eq!(
            decoded.rgba,
            vec![
                10, 20, 30, 0, 200, 100, 50, 128, 255, 255, 255, 255, // row 0
                255, 255, 255, 255, 200, 100, 50, 128, 10, 20, 30, 0, // row 1
            ]
        );
    }

    #[test]
    fn inflate_fixed_huffman_stream() {
        // Compress "HelloHello" with Fixed Huffman (BTYPE=01):
        // 'H'=72, 'e'=101, 'l'=108, 'l'=108, 'o'=111, then length=5 (sym 259, code 0000011 -> 7 bits),
        // distance=5 (sym 4, 5-bit code 00100 + 1 extra bit 0), then EOB=256 (7 bits 0000000).
        struct BitWriter {
            bytes: Vec<u8>,
            buf: u32,
            cnt: u8,
        }
        impl BitWriter {
            fn write_bits(&mut self, val: u32, n: u8) {
                self.buf |= val << self.cnt;
                self.cnt += n;
                while self.cnt >= 8 {
                    self.bytes.push((self.buf & 0xFF) as u8);
                    self.buf >>= 8;
                    self.cnt -= 8;
                }
            }
            fn write_huff_msb(&mut self, code: u32, n: u8) {
                for bit in (0..n).rev() {
                    self.write_bits((code >> bit) & 1, 1);
                }
            }
            fn finish(mut self) -> Vec<u8> {
                if self.cnt > 0 {
                    self.bytes.push((self.buf & 0xFF) as u8);
                }
                self.bytes
            }
        }

        let mut bw = BitWriter {
            bytes: vec![0x78, 0x9C],
            buf: 0,
            cnt: 0,
        };
        bw.write_bits(1, 1); // BFINAL=1
        bw.write_bits(1, 2); // BTYPE=01 (Fixed Huffman)
        for &ch in b"Hello" {
            // Fixed Huffman: 0..=143 use 8-bit codes starting at 0b00110000 (48)
            bw.write_huff_msb(48 + ch as u32, 8);
        }
        // Length 5 -> symbol 259 (256..=279 use 7-bit codes starting at 0)
        bw.write_huff_msb(259 - 256, 7);
        // Distance 5 -> symbol 4 (base=5, 1 extra bit = 0)
        bw.write_huff_msb(4, 5);
        bw.write_bits(0, 1);
        // EOB 256
        bw.write_huff_msb(0, 7);
        let mut z = bw.finish();
        z.extend_from_slice(&adler32(b"HelloHello").to_be_bytes());

        let out = inflate_zlib(&z, 64).expect("fixed huffman stream should inflate");
        assert_eq!(&out, b"HelloHello");
    }

    #[test]
    fn inflate_dynamic_huffman_stream() {
        let expected: Vec<u8> = (0..400)
            .map(|i| ((i * 13 + (i >> 3) * 7) % 17) as u8)
            .collect();
        let dyn_zlib: [u8; 67] = [
            120, 218, 237, 206, 201, 13, 0, 32, 12, 3, 193, 112, 132, 59, 129, 254, 171, 69, 194,
            46, 131, 207, 252, 188, 178, 88, 215, 224, 163, 192, 184, 225, 172, 48, 29, 184, 26,
            204, 2, 173, 67, 13, 208, 7, 51, 145, 153, 201, 76, 98, 102, 49, 147, 153, 49, 102,
            148, 25, 151, 255, 227, 77, 47, 201, 23, 12, 139,
        ];
        assert_eq!((dyn_zlib[2] >> 1) & 3, 2);
        let out = inflate_zlib(&dyn_zlib, expected.len()).expect("dynamic huffman must inflate");
        assert_eq!(out, expected);
    }

    #[test]
    fn bmp_24_bottom_up_and_32_top_down() {
        // 2x2 24-bit bottom-up BMP (each row 6 bytes + 2 padding = 8 bytes)
        let mut bmp24 = vec![0u8; 54 + 16];
        bmp24[0..2].copy_from_slice(b"BM");
        bmp24[10..14].copy_from_slice(&54u32.to_le_bytes());
        bmp24[14..18].copy_from_slice(&40u32.to_le_bytes());
        bmp24[18..22].copy_from_slice(&2i32.to_le_bytes());
        bmp24[22..26].copy_from_slice(&2i32.to_le_bytes()); // bottom-up
        bmp24[26..28].copy_from_slice(&1u16.to_le_bytes());
        bmp24[28..30].copy_from_slice(&24u16.to_le_bytes());
        // Bottom row (y=1): blue, white
        bmp24[54..60].copy_from_slice(&[255, 0, 0, 255, 255, 255]);
        // Top row (y=0): red, green
        bmp24[62..68].copy_from_slice(&[0, 0, 255, 0, 255, 0]);

        let img24 = decode(&bmp24).expect("24-bit BMP must decode");
        assert_eq!(img24.width, 2);
        assert_eq!(img24.height, 2);
        assert_eq!(
            img24.rgba,
            vec![
                255, 0, 0, 255, 0, 255, 0, 255, // y=0
                0, 0, 255, 255, 255, 255, 255, 255, // y=1
            ]
        );

        // 2x1 32-bit top-down BI_BITFIELDS BMP with custom ARGB masks
        let mut bmp32 = vec![0u8; 70 + 8];
        bmp32[0..2].copy_from_slice(b"BM");
        bmp32[10..14].copy_from_slice(&70u32.to_le_bytes());
        bmp32[14..18].copy_from_slice(&56u32.to_le_bytes());
        bmp32[18..22].copy_from_slice(&2i32.to_le_bytes());
        bmp32[22..26].copy_from_slice(&(-1i32).to_le_bytes()); // top-down
        bmp32[26..28].copy_from_slice(&1u16.to_le_bytes());
        bmp32[28..30].copy_from_slice(&32u16.to_le_bytes());
        bmp32[30..34].copy_from_slice(&3u32.to_le_bytes()); // BI_BITFIELDS
        bmp32[54..58].copy_from_slice(&0x00FF_0000u32.to_le_bytes()); // R
        bmp32[58..62].copy_from_slice(&0x0000_FF00u32.to_le_bytes()); // G
        bmp32[62..66].copy_from_slice(&0x0000_00FFu32.to_le_bytes()); // B
        bmp32[66..70].copy_from_slice(&0xFF00_0000u32.to_le_bytes()); // A
        bmp32[70..74].copy_from_slice(&0x8011_2233u32.to_le_bytes());
        bmp32[74..78].copy_from_slice(&0xFFAA_BBCCu32.to_le_bytes());

        let img32 = decode(&bmp32).expect("32-bit bitfields BMP must decode");
        assert_eq!(img32.rgba, vec![0x11, 0x22, 0x33, 0x80, 0xAA, 0xBB, 0xCC, 0xFF]);
    }

    #[test]
    fn ppm_p6_and_p3_exact_pixels() {
        let mut p6 = b"P6\n# binary test\n2 1\n255\n".to_vec();
        p6.extend_from_slice(&[10, 20, 30, 200, 150, 100]);
        let img6 = decode(&p6).expect("P6 PPM must decode");
        assert_eq!(img6.width, 2);
        assert_eq!(img6.height, 1);
        assert_eq!(img6.rgba, vec![10, 20, 30, 255, 200, 150, 100, 255]);

        let p3 = b"P3\n# ascii\n2 2\n255\n255 0 0  0 255 0\n0 0 255  128 128 128\n";
        let img3 = decode(p3).expect("P3 PPM must decode");
        assert_eq!(img3.width, 2);
        assert_eq!(img3.height, 2);
        assert_eq!(
            img3.rgba,
            vec![
                255, 0, 0, 255, 0, 255, 0, 255, // row 0
                0, 0, 255, 255, 128, 128, 128, 255, // row 1
            ]
        );
    }

    #[test]
    fn svg_renders_rect_circle_and_line() {
        let svg = br##"<?xml version="1.0" encoding="UTF-8"?>
        <svg width="20" height="20" xmlns="http://www.w3.org/2000/svg">
            <rect x="2" y="2" width="6" height="6" fill="#ff0000"/>
            <circle cx="14" cy="14" r="4" fill="blue"/>
            <line x1="0" y1="18" x2="20" y2="18" stroke="lime" stroke-width="2"/>
        </svg>"##;
        let img = decode(svg).expect("SVG must decode");
        assert_eq!(img.width, 20);
        assert_eq!(img.height, 20);

        let px = |x: u32, y: u32| -> &[u8] {
            let i = ((y * img.width + x) * 4) as usize;
            &img.rgba[i..i + 4]
        };
        assert_eq!(px(0, 0), [0, 0, 0, 0]);
        assert_eq!(px(4, 4), [255, 0, 0, 255]);
        assert_eq!(px(14, 14), [0, 0, 255, 255]);
        assert_eq!(px(5, 18), [0, 255, 0, 255]);
    }

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

        // Corrupt / JPEG / empty inputs
        assert!(decode(b"").is_err());
        assert!(decode(b"\xFF\xD8\xFF\xE0\x00\x10JFIF").is_err());
        assert!(decode(b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0DIHDRgarbage").is_err());
        assert!(decode(b"BMshort").is_err());
        assert!(decode(b"random unknown bytes").is_err());
    }
}
