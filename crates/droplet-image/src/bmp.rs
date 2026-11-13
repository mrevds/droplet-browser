//! BMP decoder (24-bit and 32-bit BI_RGB / BI_BITFIELDS).

use crate::{validate_dimensions, DecodedImage, ImageError};

pub(crate) fn decode(bytes: &[u8]) -> Result<DecodedImage, ImageError> {
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

#[cfg(test)]
mod tests {
    use crate::decode;

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
}
