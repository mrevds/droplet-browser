//! PNG decoder (chunks, PLTE/tRNS, scanline unfiltering, and Adam7 interlacing).

use crate::deflate::inflate_zlib;
use crate::{validate_dimensions, DecodedImage, ImageError};

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

pub(crate) fn decode(bytes: &[u8]) -> Result<DecodedImage, ImageError> {
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
        let chunk_len =
            u32::from_be_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]])
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
                if payload.is_empty() || !payload.len().is_multiple_of(3) || payload.len() > 256 * 3
                {
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

#[cfg(test)]
pub(crate) fn push_png_chunk(out: &mut Vec<u8>, chunk_type: &[u8; 4], payload: &[u8]) {
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(chunk_type);
    out.extend_from_slice(payload);
    let c = crc32(&[chunk_type, payload]);
    out.extend_from_slice(&c.to_be_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode;
    use crate::deflate::adler32;

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

    fn build_png_with_interlace(
        [width, height]: [u32; 2],
        bit_depth: u8,
        color_type: u8,
        interlace: u8,
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
        ihdr.push(interlace);
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

    fn build_png(
        width: u32,
        height: u32,
        bit_depth: u8,
        color_type: u8,
        palette: Option<&[[u8; 3]]>,
        trns: Option<&[u8]>,
        raw_scanlines: &[u8],
    ) -> Vec<u8> {
        build_png_with_interlace(
            [width, height],
            bit_depth,
            color_type,
            0,
            palette,
            trns,
            raw_scanlines,
        )
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
    fn png_adam7_interlaced_rgba() {
        let expected: [[[u8; 4]; 3]; 3] = [
            [[1, 2, 3, 255], [4, 5, 6, 255], [7, 8, 9, 255]],
            [[10, 11, 12, 255], [13, 14, 15, 255], [16, 17, 18, 255]],
            [[19, 20, 21, 255], [22, 23, 24, 255], [25, 26, 27, 255]],
        ];
        const ADAM7: [(usize, usize, usize, usize); 7] = [
            (0, 0, 8, 8),
            (4, 0, 8, 8),
            (0, 4, 4, 8),
            (2, 0, 4, 4),
            (0, 2, 2, 4),
            (1, 0, 2, 2),
            (0, 1, 1, 2),
        ];
        let mut raw = Vec::new();
        for &(x0, y0, dx, dy) in &ADAM7 {
            if 3 <= x0 || 3 <= y0 {
                continue;
            }
            for y in (y0..3).step_by(dy) {
                raw.push(0); // filter = None
                for x in (x0..3).step_by(dx) {
                    raw.extend_from_slice(&expected[y][x]);
                }
            }
        }
        let png = build_png_with_interlace([3, 3], 8, 6, 1, None, None, &raw);
        let decoded = decode(&png).expect("Adam7 interlaced PNG must decode");
        assert_eq!(decoded.width, 3);
        assert_eq!(decoded.height, 3);
        let flat_expected: Vec<u8> = expected
            .iter()
            .flat_map(|row| row.iter().flat_map(|p| *p))
            .collect();
        assert_eq!(decoded.rgba, flat_expected);
    }
}
