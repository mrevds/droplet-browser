//! PPM decoder (P6 binary and P3 ASCII).

use crate::{validate_dimensions, DecodedImage, ImageError};

pub(crate) fn decode(bytes: &[u8]) -> Result<DecodedImage, ImageError> {
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

#[cfg(test)]
mod tests {
    use crate::decode;

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
}
