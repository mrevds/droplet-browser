//! Software XRGB framebuffer and anti-aliased 2D rasterization primitives.

use std::path::Path;

use droplet_font::FontEngine;
use droplet_image::DecodedImage;

use crate::color::Color;
use crate::geometry::Rect;

/// Row-major `0x00RRGGBB` pixel framebuffer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Canvas {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u32>,
}

/// Alpha-blend `src` with additional `coverage` (`0..=255`) over an XRGB destination pixel.
#[inline]
pub fn blend_alpha(dst: u32, src: Color, coverage: u8) -> u32 {
    if coverage == 0 || src.a == 0 {
        return dst;
    }
    let a = ((src.a as u32) * (coverage as u32) + 127) / 255;
    if a == 0 {
        return dst;
    }
    if a == 255 {
        return src.to_u32_xrgb();
    }
    let inv_a = 255 - a;
    let dr = (dst >> 16) & 0xFF;
    let dg = (dst >> 8) & 0xFF;
    let db = dst & 0xFF;
    let r = ((src.r as u32) * a + dr * inv_a + 127) / 255;
    let g = ((src.g as u32) * a + dg * inv_a + 127) / 255;
    let b = ((src.b as u32) * a + db * inv_a + 127) / 255;
    (r << 16) | (g << 8) | b
}

/// Alpha-blend `src` over an XRGB destination pixel.
#[inline]
pub fn blend_pixel(dst: u32, src: Color) -> u32 {
    blend_alpha(dst, src, 255)
}

#[inline]
fn fill_span(span: &mut [u32], color: Color, solid_px: u32) {
    if color.a == 255 {
        span.fill(solid_px);
    } else {
        for px in span {
            *px = blend_pixel(*px, color);
        }
    }
}

#[inline]
fn blend_solid_pixel(dst: &mut u32, color: Color, solid_px: u32) {
    *dst = if color.a == 255 {
        solid_px
    } else {
        blend_pixel(*dst, color)
    };
}

#[inline]
fn blend_corner_pixel(
    dst: &mut u32,
    point: (f32, f32),
    center: (f32, f32),
    radius: f32,
    color: Color,
    solid_px: u32,
) {
    let d = ((point.0 - center.0).powi(2) + (point.1 - center.1).powi(2)).sqrt() - radius;
    let cov = ((0.5 - d).clamp(0.0, 1.0) * 255.0).round() as u8;
    if cov > 0 {
        *dst = if cov == 255 && color.a == 255 {
            solid_px
        } else {
            blend_alpha(*dst, color, cov)
        };
    }
}

/// Signed distance from `(px, py)` to a rounded rectangle (`<= 0` inside).
pub fn rounded_rect_sdf(px: f32, py: f32, rect: Rect, radius: f32) -> f32 {
    let hx = rect.width * 0.5;
    let hy = rect.height * 0.5;
    let r = radius.min(hx).min(hy).max(0.0);
    let cx = rect.x + hx;
    let cy = rect.y + hy;
    let dx = (px - cx).abs() - (hx - r);
    let dy = (py - cy).abs() - (hy - r);
    let out_dist = (dx.max(0.0).powi(2) + dy.max(0.0).powi(2)).sqrt();
    let in_dist = dx.max(dy).min(0.0);
    out_dist + in_dist - r
}

/// Intersect `rect` with `clip` and canvas bounds `[0, canvas_w) x [0, canvas_h)`.
pub fn rect_clip_bounds(
    rect: Rect,
    clip: Rect,
    canvas_w: usize,
    canvas_h: usize,
) -> Option<(usize, usize, usize, usize)> {
    if canvas_w == 0
        || canvas_h == 0
        || rect.width <= 0.0
        || rect.height <= 0.0
        || clip.width <= 0.0
        || clip.height <= 0.0
    {
        return None;
    }
    if !rect.x.is_finite()
        || !rect.y.is_finite()
        || !rect.width.is_finite()
        || !rect.height.is_finite()
        || !clip.x.is_finite()
        || !clip.y.is_finite()
        || !clip.width.is_finite()
        || !clip.height.is_finite()
    {
        return None;
    }

    let cw = canvas_w as f64;
    let ch = canvas_h as f64;
    let left = (rect.x as f64).max(clip.x as f64).max(0.0);
    let top = (rect.y as f64).max(clip.y as f64).max(0.0);
    let right = ((rect.x as f64) + (rect.width as f64))
        .min((clip.x as f64) + (clip.width as f64))
        .min(cw);
    let bottom = ((rect.y as f64) + (rect.height as f64))
        .min((clip.y as f64) + (clip.height as f64))
        .min(ch);

    if left >= right || top >= bottom {
        return None;
    }

    let x0 = (left.round() as usize).min(canvas_w);
    let y0 = (top.round() as usize).min(canvas_h);
    let x1 = (right.round() as usize).min(canvas_w);
    let y1 = (bottom.round() as usize).min(canvas_h);
    if x0 >= x1 || y0 >= y1 {
        return None;
    }
    Some((x0, y0, x1, y1))
}

/// Draw an axis-aligned rectangular outline clipped to `clip`.
pub fn draw_rect_outline(
    canvas: &mut Canvas,
    rect: Rect,
    thickness: f32,
    color: Color,
    clip: Rect,
) {
    if rect.width <= 0.0 || rect.height <= 0.0 || !thickness.is_finite() || thickness <= 0.0 {
        return;
    }
    canvas.fill_rect_clipped(
        Rect {
            x: rect.x,
            y: rect.y,
            width: rect.width,
            height: thickness,
        },
        clip,
        color,
    );
    canvas.fill_rect_clipped(
        Rect {
            x: rect.x,
            y: rect.y + rect.height - thickness,
            width: rect.width,
            height: thickness,
        },
        clip,
        color,
    );
    canvas.fill_rect_clipped(
        Rect {
            x: rect.x,
            y: rect.y + thickness,
            width: thickness,
            height: (rect.height - 2.0 * thickness).max(0.0),
        },
        clip,
        color,
    );
    canvas.fill_rect_clipped(
        Rect {
            x: rect.x + rect.width - thickness,
            y: rect.y + thickness,
            width: thickness,
            height: (rect.height - 2.0 * thickness).max(0.0),
        },
        clip,
        color,
    );
}

/// Draw an anti-aliased line segment between `(x0, y0)` and `(x1, y1)` clipped to `clip`.
#[allow(clippy::too_many_arguments)]
pub fn draw_line_aa(
    canvas: &mut Canvas,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    thickness: f32,
    color: Color,
    clip: Rect,
) {
    if color.a == 0
        || !thickness.is_finite()
        || thickness <= 0.0
        || !x0.is_finite()
        || !y0.is_finite()
        || !x1.is_finite()
        || !y1.is_finite()
        || !canvas.has_valid_backing()
    {
        return;
    }
    let half = thickness * 0.5;
    let min_x = x0.min(x1) - half - 1.0;
    let min_y = y0.min(y1) - half - 1.0;
    let max_x = x0.max(x1) + half + 1.0;
    let max_y = y0.max(y1) + half + 1.0;
    let bbox = Rect {
        x: min_x,
        y: min_y,
        width: (max_x - min_x).max(0.0),
        height: (max_y - min_y).max(0.0),
    };
    let Some((bx0, by0, bx1, by1)) = rect_clip_bounds(bbox, clip, canvas.width, canvas.height)
    else {
        return;
    };

    let vx = x1 - x0;
    let vy = y1 - y0;
    let len_sq = vx * vx + vy * vy;

    for row in by0..by1 {
        let py = row as f32 + 0.5;
        let row_offset = row * canvas.width;
        for col in bx0..bx1 {
            let px = col as f32 + 0.5;
            let dist = if len_sq <= 1e-6 {
                ((px - x0).powi(2) + (py - y0).powi(2)).sqrt()
            } else {
                let t = (((px - x0) * vx + (py - y0) * vy) / len_sq).clamp(0.0, 1.0);
                let proj_x = x0 + t * vx;
                let proj_y = y0 + t * vy;
                ((px - proj_x).powi(2) + (py - proj_y).powi(2)).sqrt()
            };
            let cov = ((half + 0.5 - dist).clamp(0.0, 1.0) * 255.0).round() as u8;
            if cov > 0 {
                let idx = row_offset + col;
                canvas.pixels[idx] = blend_alpha(canvas.pixels[idx], color, cov);
            }
        }
    }
}

impl Canvas {
    /// Allocate a `width x height` canvas filled with `bg`.
    pub fn new(width: usize, height: usize, bg: Color) -> Self {
        let len = width.saturating_mul(height);
        Self {
            width,
            height,
            pixels: vec![bg.to_u32_xrgb(); len],
        }
    }

    #[inline]
    fn has_valid_backing(&self) -> bool {
        self.width
            .checked_mul(self.height)
            .is_some_and(|len| self.pixels.len() >= len)
    }

    /// Resize the framebuffer in place and clear all pixels to `bg`.
    pub fn resize(&mut self, width: usize, height: usize, bg: Color) {
        self.width = width;
        self.height = height;
        let len = width.saturating_mul(height);
        let fill_px = bg.to_u32_xrgb();
        self.pixels.resize(len, fill_px);
        self.pixels.fill(fill_px);
    }

    /// Blend or fill the entire canvas with `color`.
    pub fn fill(&mut self, color: Color) {
        if color.a == 0 || self.pixels.is_empty() {
            return;
        }
        fill_span(&mut self.pixels, color, color.to_u32_xrgb());
    }

    /// Fill an axis-aligned rectangle clipped only to the canvas bounds.
    pub fn fill_rect(&mut self, rect: Rect, color: Color) {
        let full = Rect {
            x: 0.0,
            y: 0.0,
            width: self.width as f32,
            height: self.height as f32,
        };
        self.fill_rect_clipped(rect, full, color);
    }

    /// Fill an axis-aligned rectangle clipped to `clip`.
    pub fn fill_rect_clipped(&mut self, rect: Rect, clip: Rect, color: Color) {
        if color.a == 0 || !self.has_valid_backing() {
            return;
        }
        let Some((x0, y0, x1, y1)) = rect_clip_bounds(rect, clip, self.width, self.height) else {
            return;
        };

        let solid_px = color.to_u32_xrgb();
        for row in y0..y1 {
            let start = row * self.width + x0;
            let end = row * self.width + x1;
            fill_span(&mut self.pixels[start..end], color, solid_px);
        }
    }

    /// Fill a rounded rectangle with anti-aliased corners clipped to `clip`.
    pub fn fill_rounded_rect_clipped(
        &mut self,
        rect: Rect,
        radius: f32,
        clip: Rect,
        color: Color,
    ) {
        if color.a == 0 || !self.has_valid_backing() {
            return;
        }
        if !radius.is_finite() || radius <= 0.5 {
            self.fill_rect_clipped(rect, clip, color);
            return;
        }
        let r = radius.min(rect.width * 0.5).min(rect.height * 0.5).max(0.0);
        if r <= 0.5 {
            self.fill_rect_clipped(rect, clip, color);
            return;
        }

        let Some((x0, y0, x1, y1)) = rect_clip_bounds(rect, clip, self.width, self.height) else {
            return;
        };

        let top_band_end = ((rect.y + r).ceil() as isize).clamp(y0 as isize, y1 as isize) as usize;
        let bot_band_start =
            ((rect.y + rect.height - r).floor() as isize).clamp(y0 as isize, y1 as isize) as usize;
        let left_corner_end =
            ((rect.x + r).ceil() as isize).clamp(x0 as isize, x1 as isize) as usize;
        let right_corner_start =
            ((rect.x + rect.width - r).floor() as isize).clamp(x0 as isize, x1 as isize) as usize;

        let solid_px = color.to_u32_xrgb();
        let tl_cx = rect.x + r;
        let tl_cy = rect.y + r;
        let tr_cx = rect.x + rect.width - r;
        let bl_cy = rect.y + rect.height - r;

        for row in y0..y1 {
            let row_offset = row * self.width;
            let in_top = row < top_band_end;
            let in_bot = row >= bot_band_start;
            let py = row as f32 + 0.5;

            let cy = if in_top && py < tl_cy {
                tl_cy
            } else if in_bot && py > bl_cy {
                bl_cy
            } else {
                // Row falls strictly inside the straight vertical range.
                fill_span(
                    &mut self.pixels[row_offset + x0..row_offset + x1],
                    color,
                    solid_px,
                );
                continue;
            };

            for col in x0..left_corner_end {
                let px = col as f32 + 0.5;
                let dst = &mut self.pixels[row_offset + col];
                if px < tl_cx {
                    blend_corner_pixel(dst, (px, py), (tl_cx, cy), r, color, solid_px);
                } else if px > tr_cx {
                    blend_corner_pixel(dst, (px, py), (tr_cx, cy), r, color, solid_px);
                } else {
                    blend_solid_pixel(dst, color, solid_px);
                }
            }

            let mid_start = left_corner_end;
            let mid_end = right_corner_start.max(mid_start);
            if mid_start < mid_end {
                fill_span(
                    &mut self.pixels[row_offset + mid_start..row_offset + mid_end],
                    color,
                    solid_px,
                );
            }

            let r_start = right_corner_start.max(left_corner_end);
            for col in r_start..x1 {
                let px = col as f32 + 0.5;
                let dst = &mut self.pixels[row_offset + col];
                if px > tr_cx {
                    blend_corner_pixel(dst, (px, py), (tr_cx, cy), r, color, solid_px);
                } else {
                    blend_solid_pixel(dst, color, solid_px);
                }
            }
        }
    }

    /// Draw an anti-aliased rounded rectangle outline clipped to `clip`.
    pub fn draw_rounded_outline_clipped(
        &mut self,
        rect: Rect,
        radius: f32,
        thickness: f32,
        clip: Rect,
        color: Color,
    ) {
        if color.a == 0 || !thickness.is_finite() || thickness <= 0.0 || !self.has_valid_backing()
        {
            return;
        }
        let r = if radius.is_finite() {
            radius.min(rect.width * 0.5).min(rect.height * 0.5).max(0.0)
        } else {
            0.0
        };
        if r <= 0.5 {
            draw_rect_outline(self, rect, thickness, color, clip);
            return;
        }

        let Some((x0, y0, x1, y1)) = rect_clip_bounds(rect, clip, self.width, self.height) else {
            return;
        };

        let t = thickness.min(rect.width * 0.5).min(rect.height * 0.5);
        let inner = Rect {
            x: rect.x + t,
            y: rect.y + t,
            width: (rect.width - 2.0 * t).max(0.0),
            height: (rect.height - 2.0 * t).max(0.0),
        };
        let inner_r = (r - t).max(0.0);

        let margin = r.max(t).ceil() as usize + 1;
        let skip_x0 = (x0 + margin).min(x1);
        let skip_x1 = x1.saturating_sub(margin).max(skip_x0);
        let skip_y0 = (y0 + margin).min(y1);
        let skip_y1 = y1.saturating_sub(margin).max(skip_y0);

        for row in y0..y1 {
            let py = row as f32 + 0.5;
            let row_offset = row * self.width;
            let in_mid_y = row >= skip_y0 && row < skip_y1;

            for col in x0..x1 {
                if in_mid_y && col >= skip_x0 && col < skip_x1 {
                    continue;
                }
                let px = col as f32 + 0.5;
                let d_out = rounded_rect_sdf(px, py, rect, r);
                let cov_out = (0.5 - d_out).clamp(0.0, 1.0);
                if cov_out <= 0.0 {
                    continue;
                }
                let cov_in = if inner.width > 0.0 && inner.height > 0.0 {
                    let d_in = rounded_rect_sdf(px, py, inner, inner_r);
                    (0.5 + d_in).clamp(0.0, 1.0)
                } else {
                    1.0
                };
                let cov = (cov_out.min(cov_in) * 255.0).round() as u8;
                if cov > 0 {
                    let idx = row_offset + col;
                    self.pixels[idx] = blend_alpha(self.pixels[idx], color, cov);
                }
            }
        }
    }

    /// Rasterize anti-aliased text starting at line-box top-left `(x, y)` clipped to `clip`.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_text_aa(
        &mut self,
        font: &mut FontEngine,
        x: f32,
        y: f32,
        text: &str,
        color: Color,
        font_size: f32,
        bold: bool,
        monospace: bool,
        clip: Rect,
    ) {
        if color.a == 0
            || text.is_empty()
            || self.width == 0
            || self.height == 0
            || !self.has_valid_backing()
            || !font_size.is_finite()
            || font_size <= 0.0
            || clip.width <= 0.0
            || clip.height <= 0.0
        {
            return;
        }
        if !x.is_finite()
            || !y.is_finite()
            || !clip.x.is_finite()
            || !clip.y.is_finite()
            || !clip.width.is_finite()
            || !clip.height.is_finite()
        {
            return;
        }

        let clip_left = (clip.x as f64).max(0.0);
        let clip_top = (clip.y as f64).max(0.0);
        let clip_right = ((clip.x as f64) + (clip.width as f64)).min(self.width as f64);
        let clip_bottom = ((clip.y as f64) + (clip.height as f64)).min(self.height as f64);
        if clip_left >= clip_right || clip_top >= clip_bottom {
            return;
        }

        let cx0 = (clip_left.round() as isize).clamp(0, self.width as isize);
        let cy0 = (clip_top.round() as isize).clamp(0, self.height as isize);
        let cx1 = (clip_right.round() as isize).clamp(0, self.width as isize);
        let cy1 = (clip_bottom.round() as isize).clamp(0, self.height as isize);
        if cx0 >= cx1 || cy0 >= cy1 {
            return;
        }

        if (y + font_size * 1.6) < (cy0 as f32) || (y - font_size * 0.6) > (cy1 as f32) {
            return;
        }
        if x > (cx1 as f32) + font_size {
            return;
        }

        let mut pen_x = x;

        for ch in text.chars() {
            if pen_x > (cx1 as f32) + font_size {
                break;
            }
            let glyph = font.rasterize_glyph(ch, font_size, bold, monospace);
            let gw = glyph.width as usize;
            let gh = glyph.height as usize;
            let bx = glyph.bearing_x as f32;
            let by = glyph.bearing_y as f32;
            let advance = glyph.advance;
            let alpha = &glyph.alpha;

            if gw > 0 && gh > 0 && alpha.len() >= gw * gh {
                let gx = (pen_x + bx).round() as isize;
                let gy = (y + by).round() as isize;
                let x_start = gx.max(cx0);
                let x_end = gx.saturating_add(gw as isize).min(cx1);
                let y_start = gy.max(cy0);
                let y_end = gy.saturating_add(gh as isize).min(cy1);

                if x_start < x_end && y_start < y_end {
                    for sy in y_start..y_end {
                        let row_g = (sy - gy) as usize * gw;
                        let row_s = (sy as usize) * self.width;
                        for sx in x_start..x_end {
                            let cov = alpha[row_g + (sx - gx) as usize];
                            if cov != 0 {
                                let idx = row_s + sx as usize;
                                self.pixels[idx] = blend_alpha(self.pixels[idx], color, cov);
                            }
                        }
                    }
                }
            }

            pen_x += advance;
        }
    }

    /// Draw a text underline positioned relative to the typographic baseline.
    pub fn draw_underline(
        &mut self,
        x: f32,
        y: f32,
        width: f32,
        font_size: f32,
        color: Color,
        clip: Rect,
    ) {
        if !font_size.is_finite() {
            return;
        }
        let fs = font_size.max(1.0);
        let underline_y = y + droplet_font::baseline_offset_px(fs) + (fs * 0.09).max(1.0);
        let thickness = (fs / 14.0).clamp(1.0, 3.0);
        self.fill_rect_clipped(
            Rect {
                x,
                y: underline_y,
                width,
                height: thickness,
            },
            clip,
            color,
        );
    }

    /// Draw an RGBA8 `DecodedImage` into `rect` clipped to `clip` (1:1 copy or bilinear scale).
    pub fn draw_image_clipped(&mut self, rect: Rect, image: &DecodedImage, clip: Rect) {
        if image.width == 0 || image.height == 0 || !self.has_valid_backing() {
            return;
        }
        let Some(pixel_count) = (image.width as usize).checked_mul(image.height as usize) else {
            return;
        };
        let Some(expected_len) = pixel_count.checked_mul(4) else {
            return;
        };
        if image.rgba.len() < expected_len {
            return;
        }
        let Some((x0, y0, x1, y1)) = rect_clip_bounds(rect, clip, self.width, self.height) else {
            return;
        };

        let iw = image.width as usize;
        let ih = image.height as usize;

        // Fast exact 1:1 pixel copy / alpha blend
        if (rect.width - iw as f32).abs() < 0.01
            && (rect.height - ih as f32).abs() < 0.01
            && (rect.x - rect.x.round()).abs() < 0.01
            && (rect.y - rect.y.round()).abs() < 0.01
        {
            let rx = rect.x.round() as isize;
            let ry = rect.y.round() as isize;
            for sy in y0..y1 {
                let iy = sy as isize - ry;
                if iy < 0 || iy >= ih as isize {
                    continue;
                }
                let src_row = (iy as usize) * iw;
                let dst_row = sy * self.width;
                for sx in x0..x1 {
                    let ix = sx as isize - rx;
                    if ix < 0 || ix >= iw as isize {
                        continue;
                    }
                    let si = (src_row + ix as usize) * 4;
                    let a = image.rgba[si + 3];
                    if a == 0 {
                        continue;
                    }
                    let c = Color::rgba(image.rgba[si], image.rgba[si + 1], image.rgba[si + 2], a);
                    let di = dst_row + sx;
                    self.pixels[di] = if a == 255 {
                        c.to_u32_xrgb()
                    } else {
                        blend_pixel(self.pixels[di], c)
                    };
                }
            }
            return;
        }

        // Bilinear scaling with premultiplied alpha weighting to prevent dark edge halos
        let iw_f = iw as f32;
        let ih_f = ih as f32;
        for sy in y0..y1 {
            let v = (((sy as f32 + 0.5) - rect.y) / rect.height) * ih_f - 0.5;
            let v_clamped = v.clamp(0.0, (ih - 1) as f32);
            let iy0 = v_clamped.floor() as usize;
            let iy1 = (iy0 + 1).min(ih - 1);
            let fy = v_clamped - iy0 as f32;
            let fy0 = 1.0 - fy;
            let dst_row = sy * self.width;

            for sx in x0..x1 {
                let u = (((sx as f32 + 0.5) - rect.x) / rect.width) * iw_f - 0.5;
                let u_clamped = u.clamp(0.0, (iw - 1) as f32);
                let ix0 = u_clamped.floor() as usize;
                let ix1 = (ix0 + 1).min(iw - 1);
                let fx = u_clamped - ix0 as f32;
                let fx0 = 1.0 - fx;

                let w00 = fx0 * fy0;
                let w10 = fx * fy0;
                let w01 = fx0 * fy;
                let w11 = fx * fy;

                let i00 = (iy0 * iw + ix0) * 4;
                let i10 = (iy0 * iw + ix1) * 4;
                let i01 = (iy1 * iw + ix0) * 4;
                let i11 = (iy1 * iw + ix1) * 4;

                let a00 = (image.rgba[i00 + 3] as f32) / 255.0;
                let a10 = (image.rgba[i10 + 3] as f32) / 255.0;
                let a01 = (image.rgba[i01 + 3] as f32) / 255.0;
                let a11 = (image.rgba[i11 + 3] as f32) / 255.0;

                let out_a = w00 * a00 + w10 * a10 + w01 * a01 + w11 * a11;
                if out_a <= 0.001 {
                    continue;
                }

                let pr = w00 * (image.rgba[i00] as f32) * a00
                    + w10 * (image.rgba[i10] as f32) * a10
                    + w01 * (image.rgba[i01] as f32) * a01
                    + w11 * (image.rgba[i11] as f32) * a11;
                let pg = w00 * (image.rgba[i00 + 1] as f32) * a00
                    + w10 * (image.rgba[i10 + 1] as f32) * a10
                    + w01 * (image.rgba[i01 + 1] as f32) * a01
                    + w11 * (image.rgba[i11 + 1] as f32) * a11;
                let pb = w00 * (image.rgba[i00 + 2] as f32) * a00
                    + w10 * (image.rgba[i10 + 2] as f32) * a10
                    + w01 * (image.rgba[i01 + 2] as f32) * a01
                    + w11 * (image.rgba[i11 + 2] as f32) * a11;

                let di = dst_row + sx;
                if out_a >= 0.998 {
                    let r = pr.round().clamp(0.0, 255.0) as u32;
                    let g = pg.round().clamp(0.0, 255.0) as u32;
                    let b = pb.round().clamp(0.0, 255.0) as u32;
                    self.pixels[di] = (r << 16) | (g << 8) | b;
                } else {
                    let dst = self.pixels[di];
                    let inv_a = 1.0 - out_a;
                    let dr = ((dst >> 16) & 0xFF) as f32;
                    let dg = ((dst >> 8) & 0xFF) as f32;
                    let db = (dst & 0xFF) as f32;
                    let r = (pr + dr * inv_a).round().clamp(0.0, 255.0) as u32;
                    let g = (pg + dg * inv_a).round().clamp(0.0, 255.0) as u32;
                    let b = (pb + db * inv_a).round().clamp(0.0, 255.0) as u32;
                    self.pixels[di] = (r << 16) | (g << 8) | b;
                }
            }
        }
    }

    /// Encode the framebuffer as binary PPM (`P6`).
    pub fn to_ppm_bytes(&self) -> Vec<u8> {
        let header = format!("P6\n{} {}\n255\n", self.width, self.height);
        let mut out = Vec::with_capacity(header.len() + self.pixels.len() * 3);
        out.extend_from_slice(header.as_bytes());
        for &px in &self.pixels {
            out.push(((px >> 16) & 0xFF) as u8);
            out.push(((px >> 8) & 0xFF) as u8);
            out.push((px & 0xFF) as u8);
        }
        out
    }

    /// Alias for [`Canvas::to_ppm_bytes`].
    pub fn to_ppm(&self) -> Vec<u8> {
        self.to_ppm_bytes()
    }

    /// Write the framebuffer as a binary PPM (`P6`) file at `path`.
    pub fn write_ppm(&self, path: &Path) -> std::io::Result<()> {
        std::fs::write(path, self.to_ppm_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canvas_clipping_and_negative_bounds_never_panic() {
        let mut canvas = Canvas::new(40, 30, Color::rgb(255, 255, 255));
        let mut font = FontEngine::new();
        let full = Rect {
            x: 0.0,
            y: 0.0,
            width: 40.0,
            height: 30.0,
        };
        canvas.fill_rect_clipped(
            Rect {
                x: -50.0,
                y: -20.0,
                width: 30.0,
                height: 10.0,
            },
            full,
            Color::rgb(255, 0, 0),
        );
        canvas.fill_rect_clipped(
            Rect {
                x: -5.0,
                y: -5.0,
                width: 10.0,
                height: 10.0,
            },
            full,
            Color::rgb(0, 0, 0),
        );
        canvas.fill_rect_clipped(
            Rect {
                x: 35.0,
                y: 25.0,
                width: 100.0,
                height: 100.0,
            },
            full,
            Color::rgb(10, 20, 30),
        );
        canvas.fill_rect_clipped(
            Rect {
                x: 5.0,
                y: 5.0,
                width: 0.0,
                height: 10.0,
            },
            full,
            Color::rgb(255, 0, 0),
        );
        canvas.fill_rect_clipped(
            Rect {
                x: f32::NAN,
                y: 0.0,
                width: 10.0,
                height: 10.0,
            },
            full,
            Color::rgb(255, 0, 0),
        );

        assert_eq!(canvas.pixels[0], 0x0000_0000);
        assert_eq!(canvas.pixels[29 * 40 + 39], 0x000A_141E);
        assert_eq!(canvas.pixels[15 * 40 + 15], 0x00FF_FFFF);

        canvas.draw_text_aa(
            &mut font,
            -20.0,
            -4.0,
            "Hello Привет!",
            Color::rgb(1, 2, 3),
            16.0,
            true,
            false,
            full,
        );
        canvas.draw_text_aa(
            &mut font,
            -4.0,
            -4.0,
            "Привет!",
            Color::rgb(1, 2, 3),
            24.0,
            true,
            true,
            full,
        );
        canvas.draw_text_aa(
            &mut font,
            35.0,
            20.0,
            "Overflow",
            Color::rgb(1, 2, 3),
            24.0,
            false,
            false,
            full,
        );
    }

    #[test]
    fn canvas_alpha_blending_works() {
        let mut canvas = Canvas::new(4, 4, Color::rgb(0, 0, 0));
        let full = Rect {
            x: 0.0,
            y: 0.0,
            width: 4.0,
            height: 4.0,
        };
        canvas.fill_rect_clipped(full, full, Color::rgba(255, 255, 255, 0));
        assert_eq!(canvas.pixels[0], 0x0000_0000);

        canvas.fill_rect_clipped(
            Rect {
                x: 0.0,
                y: 0.0,
                width: 2.0,
                height: 2.0,
            },
            full,
            Color::rgba(255, 255, 255, 128),
        );
        let px = canvas.pixels[0];
        let r = (px >> 16) & 0xFF;
        assert!((127..=129).contains(&r), "Expected ~128, got {r}");
        assert_eq!(canvas.pixels[3], 0x0000_0000);
    }

    #[test]
    fn rounded_rect_carves_corners_and_fills_interior() {
        let mut canvas = Canvas::new(20, 20, Color::rgb(255, 255, 255));
        let full = Rect {
            x: 0.0,
            y: 0.0,
            width: 20.0,
            height: 20.0,
        };
        canvas.fill_rounded_rect_clipped(full, 6.0, full, Color::rgb(0, 0, 0));

        // Top-left corner outside arc remains white
        assert_eq!(canvas.pixels[0], 0x00FF_FFFF);
        // Bottom-right corner outside arc remains white
        assert_eq!(canvas.pixels[19 * 20 + 19], 0x00FF_FFFF);
        // Top middle edge & center interior are filled black
        assert_eq!(canvas.pixels[10], 0x0000_0000);
        assert_eq!(canvas.pixels[10 * 20 + 10], 0x0000_0000);
    }

    #[test]
    fn outline_line_aa_and_underline_render_cleanly() {
        let mut canvas = Canvas::new(30, 30, Color::rgb(255, 255, 255));
        let full = Rect {
            x: 0.0,
            y: 0.0,
            width: 30.0,
            height: 30.0,
        };
        canvas.draw_rounded_outline_clipped(
            Rect {
                x: 2.0,
                y: 2.0,
                width: 26.0,
                height: 26.0,
            },
            6.0,
            2.0,
            full,
            Color::rgb(0, 0, 0),
        );
        assert_eq!(canvas.pixels[15 * 30 + 15], 0x00FF_FFFF);
        assert_ne!(canvas.pixels[2 * 30 + 15], 0x00FF_FFFF);

        draw_line_aa(
            &mut canvas,
            4.0,
            15.0,
            25.0,
            15.0,
            2.0,
            Color::rgb(255, 0, 0),
            full,
        );
        assert_ne!(canvas.pixels[15 * 30 + 15], 0x00FF_FFFF);

        // Non-finite inputs must not panic
        draw_line_aa(
            &mut canvas,
            f32::NAN,
            0.0,
            10.0,
            10.0,
            2.0,
            Color::rgb(0, 0, 0),
            full,
        );

        let mut u_canvas = Canvas::new(40, 30, Color::rgb(255, 255, 255));
        let u_full = Rect {
            x: 0.0,
            y: 0.0,
            width: 40.0,
            height: 30.0,
        };
        u_canvas.draw_underline(4.0, 2.0, 20.0, 16.0, Color::rgb(0, 0, 255), u_full);
        assert!(u_canvas.pixels.contains(&0x0000_00FF));
    }

    #[test]
    fn draw_image_clipped_blits_and_blends_rgba() {
        let mut canvas = Canvas::new(6, 6, Color::rgb(255, 255, 255));
        let full = Rect {
            x: 0.0,
            y: 0.0,
            width: 6.0,
            height: 6.0,
        };
        let img = DecodedImage {
            width: 2,
            height: 2,
            rgba: vec![
                255, 0, 0, 255, // (0,0) opaque red
                0, 0, 255, 128, // (1,0) semi-transparent blue
                0, 255, 0, 0,   // (0,1) fully transparent
                10, 20, 30, 255, // (1,1) opaque dark
            ],
        };
        canvas.draw_image_clipped(
            Rect {
                x: 1.0,
                y: 1.0,
                width: 2.0,
                height: 2.0,
            },
            &img,
            full,
        );

        assert_eq!(canvas.pixels[6 + 1], 0x00FF_0000);
        let blended = canvas.pixels[6 + 2];
        let br = (blended >> 16) & 0xFF;
        let bb = blended & 0xFF;
        assert!((126..=129).contains(&br), "Expected blended red ~127, got {br}");
        assert_eq!(bb, 255);
        assert_eq!(canvas.pixels[2 * 6 + 1], 0x00FF_FFFF);
        assert_eq!(canvas.pixels[2 * 6 + 2], 0x000A_141E);
    }

    #[test]
    fn ppm_header_and_byte_size() {
        let mut canvas = Canvas::new(3, 2, Color::rgb(1, 2, 3));
        canvas.pixels[0] = 0x00FF_8040;
        let ppm = canvas.to_ppm();
        assert_eq!(ppm, canvas.to_ppm_bytes());
        let header = b"P6\n3 2\n255\n";
        assert!(ppm.starts_with(header));
        assert_eq!(ppm.len(), header.len() + 3 * 2 * 3);
        assert_eq!(&ppm[header.len()..header.len() + 3], &[0xFF, 0x80, 0x40]);
    }

    #[test]
    fn baseline_alignment_keeps_period_below_capitals() {
        let mut font = FontEngine::new();
        if !font.has_system_font() {
            return;
        }
        let clip = Rect {
            x: 0.0,
            y: 0.0,
            width: 120.0,
            height: 60.0,
        };
        let mut h_canvas = Canvas::new(120, 60, Color::rgb(255, 255, 255));
        h_canvas.draw_text_aa(
            &mut font,
            4.0,
            4.0,
            "H",
            Color::rgb(0, 0, 0),
            24.0,
            false,
            false,
            clip,
        );
        let mut dot_canvas = Canvas::new(120, 60, Color::rgb(255, 255, 255));
        dot_canvas.draw_text_aa(
            &mut font,
            4.0,
            4.0,
            ".",
            Color::rgb(0, 0, 0),
            24.0,
            false,
            false,
            clip,
        );

        let y_bounds = |c: &Canvas| -> (usize, usize) {
            let mut min_y = usize::MAX;
            let mut max_y = 0usize;
            for y in 0..c.height {
                for x in 0..c.width {
                    if c.pixels[y * c.width + x] != 0x00FF_FFFF {
                        min_y = min_y.min(y);
                        max_y = max_y.max(y);
                    }
                }
            }
            (min_y, max_y)
        };

        let (h_top, h_bot) = y_bounds(&h_canvas);
        let (dot_top, dot_bot) = y_bounds(&dot_canvas);
        assert!(h_top < h_bot && dot_top <= dot_bot);
        // The top of '.' must be significantly lower than the top of 'H' (near baseline, not ceiling).
        assert!(
            dot_top > h_top + 6,
            "period top {dot_top} should sit well below 'H' top {h_top}"
        );
        // Bottom of '.' sits near the baseline of 'H'.
        assert!(
            dot_bot + 2 >= h_bot && dot_bot <= h_bot + 3,
            "period bottom {dot_bot} should align near 'H' bottom {h_bot}"
        );
    }
}
