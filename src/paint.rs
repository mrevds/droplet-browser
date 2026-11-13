use std::path::Path;
use std::sync::Arc;

use crate::css::Color;
use crate::font::{self, FontEngine};
use crate::image::DecodedImage;
use crate::layout::{BoxType, LayoutBox, Rect};
use crate::style::EdgeSizes;

pub const TAB_BAR_HEIGHT: usize = 30;
pub const TOOLBAR_HEIGHT: usize = 44;
pub const CHROME_HEIGHT: usize = TAB_BAR_HEIGHT + TOOLBAR_HEIGHT; // 74
pub const STATUS_BAR_HEIGHT: usize = 0;

#[derive(Debug, Clone)]
pub struct Canvas {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u32>,
}

#[derive(Debug, Clone)]
pub struct ChromeHitboxes {
    pub back_btn: Rect,
    pub forward_btn: Rect,
    pub reload_btn: Rect,
    pub home_btn: Rect,
    pub address_bar: Rect,
    pub new_tab_btn: Rect,
    pub tab_rects: Vec<Rect>,
    pub tab_close_rects: Vec<Rect>,
}

pub fn chrome_hitboxes(window_width: usize, tab_count: usize) -> ChromeHitboxes {
    let w = window_width as f32;

    // Top tab strip (y: 0..30)
    let mut tab_rects = Vec::with_capacity(tab_count);
    let mut tab_close_rects = Vec::with_capacity(tab_count);
    let tab_y = 4.0_f32;
    let tab_h = (TAB_BAR_HEIGHT as f32) - 4.0;
    let new_tab_size = 22.0_f32;

    let new_tab_x = if tab_count == 0 {
        8.0_f32
    } else {
        let avail = (w - 16.0 - new_tab_size - 6.0).max(0.0);
        let raw_slot = avail / (tab_count as f32);
        let tab_w = (raw_slot - 4.0).clamp(16.0, 200.0);
        let step = tab_w + 4.0;
        for i in 0..tab_count {
            let tx = 8.0 + (i as f32) * step;
            let tw = tab_w.min((w - new_tab_size - 12.0 - tx).max(0.0));
            tab_rects.push(Rect {
                x: tx,
                y: tab_y,
                width: tw,
                height: tab_h,
            });
            let close_w = if tw >= 44.0 { 16.0 } else { 0.0 };
            tab_close_rects.push(Rect {
                x: tx + tw - 20.0,
                y: tab_y + 5.0,
                width: close_w,
                height: if close_w > 0.0 { 16.0 } else { 0.0 },
            });
        }
        (8.0 + (tab_count as f32) * step).min((w - new_tab_size - 8.0).max(8.0))
    };

    let new_tab_btn = Rect {
        x: new_tab_x,
        y: 5.0,
        width: new_tab_size,
        height: new_tab_size,
    };

    // Toolbar (y: 30..74)
    let btn_y = (TAB_BAR_HEIGHT as f32) + ((TOOLBAR_HEIGHT as f32) - 28.0) * 0.5;
    let btn_size = 28.0_f32;
    let back_btn = Rect {
        x: 10.0,
        y: btn_y,
        width: btn_size,
        height: btn_size,
    };
    let forward_btn = Rect {
        x: 42.0,
        y: btn_y,
        width: btn_size,
        height: btn_size,
    };
    let reload_btn = Rect {
        x: 74.0,
        y: btn_y,
        width: btn_size,
        height: btn_size,
    };
    let home_btn = Rect {
        x: 106.0,
        y: btn_y,
        width: btn_size,
        height: btn_size,
    };

    let bar_x = 144.0_f32;
    let right_reserve = if w > 320.0 { 78.0 } else { 10.0 };
    let bar_w = (w - bar_x - right_reserve).max(0.0);
    let address_bar = Rect {
        x: bar_x,
        y: (TAB_BAR_HEIGHT as f32) + 7.0,
        width: bar_w,
        height: 30.0,
    };

    ChromeHitboxes {
        back_btn,
        forward_btn,
        reload_btn,
        home_btn,
        address_bar,
        new_tab_btn,
        tab_rects,
        tab_close_rects,
    }
}

#[derive(Debug, Clone)]
pub enum DisplayCommand {
    SolidRect {
        rect: Rect,
        color: Color,
        radius: f32,
    },
    Text {
        x: f32,
        y: f32,
        text: String,
        color: Color,
        font_size: f32,
        scale: usize,
        bold: bool,
        monospace: bool,
        underline: bool,
        width: f32,
    },
    Image {
        rect: Rect,
        image: Arc<DecodedImage>,
    },
}

pub type DisplayList = Vec<DisplayCommand>;

pub struct ChromeState<'a> {
    pub url_input: &'a str,
    pub url_focused: bool,
    pub can_back: bool,
    pub can_forward: bool,
    pub status_text: &'a str,
    pub timing_badge: &'a str,
    pub tab_titles: &'a [&'a str],
    pub active_tab: usize,
    pub scroll_y: f32,
    pub content_height: f32,
}

#[inline]
fn blend_alpha(dst: u32, src: Color, coverage: u8) -> u32 {
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

#[inline]
fn blend_pixel(dst: u32, src: Color) -> u32 {
    blend_alpha(dst, src, 255)
}

#[allow(clippy::unnecessary_cast)]
fn glyph_metrics(g: &font::RasterGlyph) -> (usize, usize, f32, f32, f32, &[u8]) {
    (
        g.width as usize,
        g.height as usize,
        g.bearing_x as f32,
        g.bearing_y as f32,
        g.advance as f32,
        &g.alpha,
    )
}

fn rounded_rect_sdf(px: f32, py: f32, rect: Rect, radius: f32) -> f32 {
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

fn rect_clip_bounds(
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

impl Canvas {
    pub fn new(width: usize, height: usize, bg: Color) -> Self {
        let len = width.saturating_mul(height);
        Self {
            width,
            height,
            pixels: vec![bg.to_u32_xrgb(); len],
        }
    }

    pub fn resize(&mut self, width: usize, height: usize, bg: Color) {
        self.width = width;
        self.height = height;
        let len = width.saturating_mul(height);
        let fill_px = bg.to_u32_xrgb();
        self.pixels.resize(len, fill_px);
        self.pixels.fill(fill_px);
    }

    pub fn fill(&mut self, color: Color) {
        if color.a == 0 || self.pixels.is_empty() {
            return;
        }
        if color.a == 255 {
            self.pixels.fill(color.to_u32_xrgb());
        } else {
            for px in &mut self.pixels {
                *px = blend_pixel(*px, color);
            }
        }
    }

    pub fn fill_rect_clipped(&mut self, rect: Rect, clip: Rect, color: Color) {
        if color.a == 0 {
            return;
        }
        let Some((x0, y0, x1, y1)) = rect_clip_bounds(rect, clip, self.width, self.height) else {
            return;
        };

        if color.a == 255 {
            let pixel = color.to_u32_xrgb();
            for row in y0..y1 {
                let start = row * self.width + x0;
                let end = row * self.width + x1;
                self.pixels[start..end].fill(pixel);
            }
        } else {
            for row in y0..y1 {
                let start = row * self.width + x0;
                let end = row * self.width + x1;
                for px in &mut self.pixels[start..end] {
                    *px = blend_pixel(*px, color);
                }
            }
        }
    }

    pub fn fill_rounded_rect_clipped(
        &mut self,
        rect: Rect,
        radius: f32,
        clip: Rect,
        color: Color,
    ) {
        if color.a == 0 {
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

        let opaque = color.a == 255;
        let solid_px = color.to_u32_xrgb();

        let tl_cx = rect.x + r;
        let tl_cy = rect.y + r;
        let tr_cx = rect.x + rect.width - r;
        let bl_cy = rect.y + rect.height - r;

        for row in y0..y1 {
            let row_offset = row * self.width;
            let in_top = row < top_band_end;
            let in_bot = row >= bot_band_start;

            if !in_top && !in_bot {
                if opaque {
                    self.pixels[row_offset + x0..row_offset + x1].fill(solid_px);
                } else {
                    for px in &mut self.pixels[row_offset + x0..row_offset + x1] {
                        *px = blend_pixel(*px, color);
                    }
                }
                continue;
            }

            let py = row as f32 + 0.5;
            let cy = if in_top && py < tl_cy {
                tl_cy
            } else if in_bot && py > bl_cy {
                bl_cy
            } else {
                // Row falls strictly inside the straight vertical range
                if opaque {
                    self.pixels[row_offset + x0..row_offset + x1].fill(solid_px);
                } else {
                    for px in &mut self.pixels[row_offset + x0..row_offset + x1] {
                        *px = blend_pixel(*px, color);
                    }
                }
                continue;
            };

            // Left corner pixels
            for col in x0..left_corner_end {
                let px = col as f32 + 0.5;
                let dist = if px < tl_cx {
                    Some(((px - tl_cx).powi(2) + (py - cy).powi(2)).sqrt() - r)
                } else if px > tr_cx {
                    Some(((px - tr_cx).powi(2) + (py - cy).powi(2)).sqrt() - r)
                } else {
                    None
                };
                if let Some(d) = dist {
                    let cov = ((0.5 - d).clamp(0.0, 1.0) * 255.0).round() as u8;
                    if cov > 0 {
                        let idx = row_offset + col;
                        self.pixels[idx] = if cov == 255 && opaque {
                            solid_px
                        } else {
                            blend_alpha(self.pixels[idx], color, cov)
                        };
                    }
                } else {
                    let idx = row_offset + col;
                    self.pixels[idx] = if opaque {
                        solid_px
                    } else {
                        blend_pixel(self.pixels[idx], color)
                    };
                }
            }

            // Straight middle span between corners
            let mid_start = left_corner_end;
            let mid_end = right_corner_start.max(mid_start);
            if mid_start < mid_end {
                if opaque {
                    self.pixels[row_offset + mid_start..row_offset + mid_end].fill(solid_px);
                } else {
                    for px in &mut self.pixels[row_offset + mid_start..row_offset + mid_end] {
                        *px = blend_pixel(*px, color);
                    }
                }
            }

            // Right corner pixels
            let r_start = right_corner_start.max(left_corner_end);
            for col in r_start..x1 {
                let px = col as f32 + 0.5;
                if px > tr_cx {
                    let d = ((px - tr_cx).powi(2) + (py - cy).powi(2)).sqrt() - r;
                    let cov = ((0.5 - d).clamp(0.0, 1.0) * 255.0).round() as u8;
                    if cov > 0 {
                        let idx = row_offset + col;
                        self.pixels[idx] = if cov == 255 && opaque {
                            solid_px
                        } else {
                            blend_alpha(self.pixels[idx], color, cov)
                        };
                    }
                } else {
                    let idx = row_offset + col;
                    self.pixels[idx] = if opaque {
                        solid_px
                    } else {
                        blend_pixel(self.pixels[idx], color)
                    };
                }
            }
        }
    }

    pub fn draw_rounded_outline_clipped(
        &mut self,
        rect: Rect,
        radius: f32,
        thickness: f32,
        clip: Rect,
        color: Color,
    ) {
        if color.a == 0 || thickness <= 0.0 || !thickness.is_finite() {
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
            let (gw, gh, bx, by, advance, alpha) = glyph_metrics(glyph);

            if gw > 0 && gh > 0 && alpha.len() >= gw * gh {
                let gx = (pen_x + bx).round() as isize;
                let gy = (y + by).round() as isize;
                let x_start = gx.max(cx0);
                let x_end = (gx + gw as isize).min(cx1);
                let y_start = gy.max(cy0);
                let y_end = (gy + gh as isize).min(cy1);

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

    pub fn draw_underline(
        &mut self,
        x: f32,
        y: f32,
        width: f32,
        font_size: f32,
        color: Color,
        clip: Rect,
    ) {
        let fs = font_size.max(1.0);
        let underline_y = y + font::baseline_offset_px(fs) + (fs * 0.09).max(1.0);
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

    pub fn draw_image_clipped(&mut self, rect: Rect, image: &DecodedImage, clip: Rect) {
        if image.width == 0 || image.height == 0 {
            return;
        }
        let expected_len = (image.width as usize)
            .saturating_mul(image.height as usize)
            .saturating_mul(4);
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

    pub fn to_ppm(&self) -> Vec<u8> {
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

    pub fn write_ppm(&self, path: &Path) -> std::io::Result<()> {
        std::fs::write(path, self.to_ppm())
    }
}

pub fn build_display_list(layout_root: &LayoutBox) -> DisplayList {
    let mut list = Vec::new();
    emit_layout_box(layout_root, &mut list);
    list
}

fn emit_border_edges(bb: Rect, border: EdgeSizes, border_color: Color, list: &mut DisplayList) {
    if border.top > 0.0 {
        list.push(DisplayCommand::SolidRect {
            rect: Rect {
                x: bb.x,
                y: bb.y,
                width: bb.width,
                height: border.top,
            },
            color: border_color,
            radius: 0.0,
        });
    }
    if border.bottom > 0.0 {
        list.push(DisplayCommand::SolidRect {
            rect: Rect {
                x: bb.x,
                y: bb.y + bb.height - border.bottom,
                width: bb.width,
                height: border.bottom,
            },
            color: border_color,
            radius: 0.0,
        });
    }
    let side_y = bb.y + border.top;
    let side_h = (bb.height - border.top - border.bottom).max(0.0);
    if border.left > 0.0 && side_h > 0.0 {
        list.push(DisplayCommand::SolidRect {
            rect: Rect {
                x: bb.x,
                y: side_y,
                width: border.left,
                height: side_h,
            },
            color: border_color,
            radius: 0.0,
        });
    }
    if border.right > 0.0 && side_h > 0.0 {
        list.push(DisplayCommand::SolidRect {
            rect: Rect {
                x: bb.x + bb.width - border.right,
                y: side_y,
                width: border.right,
                height: side_h,
            },
            color: border_color,
            radius: 0.0,
        });
    }
}

fn emit_layout_box(layout_box: &LayoutBox, list: &mut DisplayList) {
    let radius = layout_box.style.border_radius.max(0.0);
    let border = layout_box.dimensions.border;
    let border_color = layout_box.style.border_color;
    let has_border = border_color.a > 0
        && (border.top > 0.0 || border.right > 0.0 || border.bottom > 0.0 || border.left > 0.0);

    let bg = match &layout_box.box_type {
        BoxType::Image { image: None, .. } => layout_box
            .style
            .background_color
            .or(Some(Color::rgb(248, 250, 252))),
        _ => layout_box.style.background_color,
    };

    let effective_border_color = match &layout_box.box_type {
        BoxType::Image { image: None, .. }
            if layout_box.style.border == EdgeSizes::default() =>
        {
            Color::rgb(203, 213, 225)
        }
        _ => border_color,
    };

    // When rounded with an opaque background and border, draw outer border box then inset padding box
    if radius > 0.5 && has_border && bg.is_some_and(|c| c.a == 255) {
        let bb = layout_box.dimensions.border_box();
        list.push(DisplayCommand::SolidRect {
            rect: bb,
            color: effective_border_color,
            radius,
        });
        let avg_b = (border.top + border.right + border.bottom + border.left) * 0.25;
        if let Some(bg_col) = bg {
            list.push(DisplayCommand::SolidRect {
                rect: layout_box.dimensions.padding_box(),
                color: bg_col,
                radius: (radius - avg_b).max(0.0),
            });
        }
    } else {
        if let Some(bg_col) = bg {
            if bg_col.a > 0 {
                list.push(DisplayCommand::SolidRect {
                    rect: layout_box.dimensions.padding_box(),
                    color: bg_col,
                    radius,
                });
            }
        }
        if has_border {
            emit_border_edges(
                layout_box.dimensions.border_box(),
                border,
                effective_border_color,
                list,
            );
        }
    }

    match &layout_box.box_type {
        BoxType::Text {
            lines,
            line_height,
            font_size,
            font_scale,
            monospace,
        } => {
            let scale = (*font_scale).max(1);
            let y_pad = ((*line_height - *font_size) * 0.5).max(0.0);
            let base_x = layout_box.dimensions.content.x;
            let base_y = layout_box.dimensions.content.y;
            for (idx, line) in lines.iter().enumerate() {
                if line.is_empty() {
                    continue;
                }
                let width = font::measure_text_px(
                    line,
                    *font_size,
                    layout_box.style.bold,
                    *monospace,
                );
                list.push(DisplayCommand::Text {
                    x: base_x,
                    y: base_y + (idx as f32) * *line_height + y_pad,
                    text: line.clone(),
                    color: layout_box.style.color,
                    font_size: *font_size,
                    scale,
                    bold: layout_box.style.bold,
                    monospace: *monospace,
                    underline: layout_box.style.text_underline,
                    width,
                });
            }
        }
        BoxType::Image { image: Some(img), .. } => {
            list.push(DisplayCommand::Image {
                rect: layout_box.dimensions.content,
                image: img.clone(),
            });
        }
        BoxType::Image { image: None, alt } => {
            let label = if alt.is_empty() { "[img]" } else { alt.as_str() };
            let fs = layout_box.style.font_size.clamp(11.0, 14.0);
            let text_w = font::measure_text_px(label, fs, false, false);
            let c = layout_box.dimensions.content;
            if c.width > 4.0 && c.height > 4.0 {
                let tx = c.x + ((c.width - text_w) * 0.5).max(0.0);
                let ty = c.y + ((c.height - fs) * 0.5).max(0.0);
                list.push(DisplayCommand::Text {
                    x: tx,
                    y: ty,
                    text: label.to_string(),
                    color: Color::rgb(100, 116, 139),
                    font_size: fs,
                    scale: 1,
                    bold: false,
                    monospace: false,
                    underline: false,
                    width: text_w,
                });
            }
        }
        _ => {}
    }

    for child in &layout_box.children {
        emit_layout_box(child, list);
    }
}

pub fn render_frame(
    canvas: &mut Canvas,
    font: &mut FontEngine,
    display_list: &[DisplayCommand],
    chrome: &ChromeState<'_>,
) {
    if canvas.width == 0 || canvas.height == 0 {
        return;
    }

    let w = canvas.width as f32;
    let chrome_h = CHROME_HEIGHT.min(canvas.height);
    let status_h = STATUS_BAR_HEIGHT;
    let viewport_h = canvas.height.saturating_sub(chrome_h).saturating_sub(status_h);

    let viewport_clip = Rect {
        x: 0.0,
        y: chrome_h as f32,
        width: w,
        height: viewport_h as f32,
    };

    if viewport_h > 0 {
        canvas.fill_rect_clipped(viewport_clip, viewport_clip, Color::rgb(255, 255, 255));

        let offset_y = CHROME_HEIGHT as f32 - chrome.scroll_y;
        let vp_top = viewport_clip.y;
        let vp_bottom = viewport_clip.y + viewport_clip.height;

        for cmd in display_list {
            match cmd {
                DisplayCommand::SolidRect {
                    rect,
                    color,
                    radius,
                } => {
                    let y = rect.y + offset_y;
                    if y + rect.height <= vp_top || y >= vp_bottom {
                        continue;
                    }
                    let shifted = Rect {
                        x: rect.x,
                        y,
                        width: rect.width,
                        height: rect.height,
                    };
                    if *radius > 0.5 {
                        canvas.fill_rounded_rect_clipped(shifted, *radius, viewport_clip, *color);
                    } else {
                        canvas.fill_rect_clipped(shifted, viewport_clip, *color);
                    }
                }
                DisplayCommand::Text {
                    x,
                    y,
                    text,
                    color,
                    font_size,
                    scale: _,
                    bold,
                    monospace,
                    underline,
                    width,
                } => {
                    let ty = *y + offset_y;
                    let th = font::line_height_px(*font_size).max(*font_size);
                    if ty + th + *font_size * 0.5 <= vp_top || ty - *font_size * 0.5 >= vp_bottom {
                        continue;
                    }
                    canvas.draw_text_aa(
                        font,
                        *x,
                        ty,
                        text,
                        *color,
                        *font_size,
                        *bold,
                        *monospace,
                        viewport_clip,
                    );
                    if *underline {
                        canvas.draw_underline(
                            *x,
                            ty,
                            *width,
                            *font_size,
                            *color,
                            viewport_clip,
                        );
                    }
                }
                DisplayCommand::Image { rect, image } => {
                    let y = rect.y + offset_y;
                    if y + rect.height <= vp_top || y >= vp_bottom {
                        continue;
                    }
                    canvas.draw_image_clipped(
                        Rect {
                            x: rect.x,
                            y,
                            width: rect.width,
                            height: rect.height,
                        },
                        image,
                        viewport_clip,
                    );
                }
            }
        }

        // Sleek floating scrollbar thumb
        if chrome.content_height > viewport_h as f32 && canvas.width >= 10 {
            let vh = viewport_h as f32;
            let thumb_h = ((vh / chrome.content_height) * vh).clamp(24.0, vh);
            let max_scroll = (chrome.content_height - vh).max(1.0);
            let ratio = (chrome.scroll_y / max_scroll).clamp(0.0, 1.0);
            let thumb_y = vp_top + ratio * (vh - thumb_h);
            let thumb = Rect {
                x: w - 8.0,
                y: thumb_y + 2.0,
                width: 5.0,
                height: (thumb_h - 4.0).max(4.0),
            };
            canvas.fill_rounded_rect_clipped(
                thumb,
                2.5,
                viewport_clip,
                Color::rgba(148, 163, 184, 175),
            );
        }

        draw_floating_status_pill(canvas, font, viewport_clip, chrome.status_text);
    }

    draw_chrome(canvas, font, chrome_h, chrome);
}

fn draw_rect_outline(canvas: &mut Canvas, rect: Rect, thickness: f32, color: Color, clip: Rect) {
    if rect.width <= 0.0 || rect.height <= 0.0 || thickness <= 0.0 {
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

#[allow(clippy::too_many_arguments)]
fn draw_line_aa(
    canvas: &mut Canvas,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    thickness: f32,
    color: Color,
    clip: Rect,
) {
    if color.a == 0 || thickness <= 0.0 {
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

#[derive(Clone, Copy)]
enum NavIcon {
    Back,
    Forward,
    Reload,
    Home,
}

fn draw_toolbar_icon_button(
    canvas: &mut Canvas,
    rect: Rect,
    icon: NavIcon,
    enabled: bool,
    clip: Rect,
) {
    let bg = if enabled {
        Color::rgb(241, 245, 249)
    } else {
        Color::rgb(248, 250, 252)
    };
    let fg = if enabled {
        Color::rgb(30, 41, 59)
    } else {
        Color::rgb(148, 163, 184)
    };

    canvas.fill_rounded_rect_clipped(rect, 6.0, clip, bg);

    let cx = rect.x + rect.width * 0.5;
    let cy = rect.y + rect.height * 0.5;
    let stroke = 1.6_f32;

    match icon {
        NavIcon::Back => {
            draw_line_aa(canvas, cx - 5.0, cy, cx + 5.0, cy, stroke, fg, clip);
            draw_line_aa(canvas, cx - 5.0, cy, cx - 0.5, cy - 4.5, stroke, fg, clip);
            draw_line_aa(canvas, cx - 5.0, cy, cx - 0.5, cy + 4.5, stroke, fg, clip);
        }
        NavIcon::Forward => {
            draw_line_aa(canvas, cx - 5.0, cy, cx + 5.0, cy, stroke, fg, clip);
            draw_line_aa(canvas, cx + 5.0, cy, cx + 0.5, cy - 4.5, stroke, fg, clip);
            draw_line_aa(canvas, cx + 5.0, cy, cx + 0.5, cy + 4.5, stroke, fg, clip);
        }
        NavIcon::Reload => {
            let r = 5.2_f32;
            let box_r = Rect {
                x: cx - r - 2.0,
                y: cy - r - 2.0,
                width: (r + 2.0) * 2.0,
                height: (r + 2.0) * 2.0,
            };
            if let Some((x0, y0, x1, y1)) = rect_clip_bounds(box_r, clip, canvas.width, canvas.height) {
                for row in y0..y1 {
                    let py = row as f32 + 0.5;
                    let row_offset = row * canvas.width;
                    for col in x0..x1 {
                        let px = col as f32 + 0.5;
                        let dx = px - cx;
                        let dy = py - cy;
                        // Leave a clean gap in upper-right quadrant
                        if dx > 1.5 && dy < -0.5 {
                            continue;
                        }
                        let dist = ((dx * dx + dy * dy).sqrt() - r).abs();
                        let cov = ((stroke * 0.5 + 0.5 - dist).clamp(0.0, 1.0) * 255.0).round() as u8;
                        if cov > 0 {
                            let idx = row_offset + col;
                            canvas.pixels[idx] = blend_alpha(canvas.pixels[idx], fg, cov);
                        }
                    }
                }
            }
            // Arrow head on top-right of loop
            draw_line_aa(canvas, cx + 1.0, cy - r - 0.2, cx + 4.5, cy - r + 1.0, stroke, fg, clip);
            draw_line_aa(canvas, cx + 4.5, cy - r + 1.0, cx + 3.2, cy - r - 2.5, stroke, fg, clip);
        }
        NavIcon::Home => {
            // Roof
            draw_line_aa(canvas, cx - 6.0, cy - 0.5, cx, cy - 6.0, stroke, fg, clip);
            draw_line_aa(canvas, cx, cy - 6.0, cx + 6.0, cy - 0.5, stroke, fg, clip);
            // Walls + base
            draw_line_aa(canvas, cx - 4.2, cy - 1.5, cx - 4.2, cy + 5.0, stroke, fg, clip);
            draw_line_aa(canvas, cx + 4.2, cy - 1.5, cx + 4.2, cy + 5.0, stroke, fg, clip);
            draw_line_aa(canvas, cx - 4.2, cy + 5.0, cx + 4.2, cy + 5.0, stroke, fg, clip);
        }
    }
}

fn truncate_prefix_to_width(
    text: &str,
    font_size: f32,
    bold: bool,
    mono: bool,
    max_w: f32,
) -> String {
    if max_w <= 0.0 || text.is_empty() {
        return String::new();
    }
    if font::measure_text_px(text, font_size, bold, mono) <= max_w {
        return text.to_string();
    }
    let ellipsis = "...";
    let ell_w = font::measure_text_px(ellipsis, font_size, bold, mono);
    if ell_w > max_w {
        let mut out = String::new();
        for ch in text.chars() {
            let mut next = out.clone();
            next.push(ch);
            if font::measure_text_px(&next, font_size, bold, mono) > max_w {
                break;
            }
            out = next;
        }
        return out;
    }
    let target = max_w - ell_w;
    let mut end_byte = 0usize;
    for (idx, ch) in text.char_indices() {
        let next = idx + ch.len_utf8();
        if font::measure_text_px(&text[..next], font_size, bold, mono) <= target {
            end_byte = next;
        } else {
            break;
        }
    }
    format!("{}...", &text[..end_byte])
}

fn tail_slice_to_width(
    text: &str,
    font_size: f32,
    bold: bool,
    mono: bool,
    max_w: f32,
) -> &str {
    if max_w <= 0.0 || text.is_empty() {
        return "";
    }
    if font::measure_text_px(text, font_size, bold, mono) <= max_w {
        return text;
    }
    for (idx, _) in text.char_indices() {
        let candidate = &text[idx..];
        if font::measure_text_px(candidate, font_size, bold, mono) <= max_w {
            return candidate;
        }
    }
    ""
}

fn draw_chrome(
    canvas: &mut Canvas,
    font: &mut FontEngine,
    chrome_h: usize,
    chrome: &ChromeState<'_>,
) {
    if chrome_h == 0 {
        return;
    }
    let w = canvas.width as f32;
    let full_chrome_clip = Rect {
        x: 0.0,
        y: 0.0,
        width: w,
        height: chrome_h as f32,
    };

    let tab_bar_h = TAB_BAR_HEIGHT.min(chrome_h) as f32;
    let tab_clip = Rect {
        x: 0.0,
        y: 0.0,
        width: w,
        height: tab_bar_h,
    };

    // Soft neutral tab strip backdrop
    canvas.fill_rect_clipped(tab_clip, tab_clip, Color::rgb(226, 232, 240));

    let hitboxes = chrome_hitboxes(canvas.width, chrome.tab_titles.len());

    for (idx, tab_rect) in hitboxes.tab_rects.iter().enumerate() {
        let is_active = idx == chrome.active_tab;
        if is_active {
            // Extend 8px downward inside tab_clip so bottom corners stay square against toolbar
            let extended = Rect {
                x: tab_rect.x,
                y: tab_rect.y,
                width: tab_rect.width,
                height: tab_rect.height + 8.0,
            };
            canvas.fill_rounded_rect_clipped(extended, 8.0, tab_clip, Color::rgb(255, 255, 255));
        } else {
            canvas.fill_rounded_rect_clipped(
                *tab_rect,
                6.0,
                tab_clip,
                Color::rgba(241, 245, 249, 170),
            );
        }

        let title = chrome.tab_titles.get(idx).copied().unwrap_or("New Tab");
        let close_rect = hitboxes.tab_close_rects.get(idx).copied().unwrap_or_default();
        let right_pad = if close_rect.width > 0.0 { 22.0 } else { 6.0 };
        let title_avail = (tab_rect.width - 10.0 - right_pad).max(0.0);
        let label = truncate_prefix_to_width(title, 12.0, is_active, false, title_avail);
        if !label.is_empty() {
            let text_clip = Rect {
                x: tab_rect.x + 8.0,
                y: tab_rect.y + 2.0,
                width: title_avail,
                height: (tab_rect.height - 4.0).max(0.0),
            };
            let ty = tab_rect.y + (tab_rect.height - font::line_height_px(12.0)) * 0.5;
            let fg = if is_active {
                Color::rgb(15, 23, 42)
            } else {
                Color::rgb(71, 85, 105)
            };
            canvas.draw_text_aa(
                font,
                text_clip.x,
                ty,
                &label,
                fg,
                12.0,
                is_active,
                false,
                text_clip,
            );
        }

        if close_rect.width > 0.0 {
            let ccx = close_rect.x + close_rect.width * 0.5;
            let ccy = close_rect.y + close_rect.height * 0.5;
            let x_col = if is_active {
                Color::rgb(100, 116, 139)
            } else {
                Color::rgb(148, 163, 184)
            };
            draw_line_aa(
                canvas,
                ccx - 3.0,
                ccy - 3.0,
                ccx + 3.0,
                ccy + 3.0,
                1.3,
                x_col,
                tab_clip,
            );
            draw_line_aa(
                canvas,
                ccx + 3.0,
                ccy - 3.0,
                ccx - 3.0,
                ccy + 3.0,
                1.3,
                x_col,
                tab_clip,
            );
        }
    }

    // New tab (+) button
    let nt = hitboxes.new_tab_btn;
    if nt.x + nt.width <= w {
        canvas.fill_rounded_rect_clipped(
            nt,
            6.0,
            tab_clip,
            Color::rgba(241, 245, 249, 180),
        );
        let ncx = nt.x + nt.width * 0.5;
        let ncy = nt.y + nt.height * 0.5;
        let plus_col = Color::rgb(71, 85, 105);
        draw_line_aa(canvas, ncx - 4.5, ncy, ncx + 4.5, ncy, 1.5, plus_col, tab_clip);
        draw_line_aa(canvas, ncx, ncy - 4.5, ncx, ncy + 4.5, 1.5, plus_col, tab_clip);
    }

    if chrome_h <= TAB_BAR_HEIGHT {
        return;
    }

    // Toolbar backdrop (#ffffff + hairline divider #e2e8f0)
    let toolbar_clip = Rect {
        x: 0.0,
        y: TAB_BAR_HEIGHT as f32,
        width: w,
        height: (chrome_h - TAB_BAR_HEIGHT) as f32,
    };
    canvas.fill_rect_clipped(toolbar_clip, full_chrome_clip, Color::rgb(255, 255, 255));
    canvas.fill_rect_clipped(
        Rect {
            x: 0.0,
            y: (CHROME_HEIGHT as f32) - 1.0,
            width: w,
            height: 1.0,
        },
        full_chrome_clip,
        Color::rgb(226, 232, 240),
    );

    draw_toolbar_icon_button(
        canvas,
        hitboxes.back_btn,
        NavIcon::Back,
        chrome.can_back,
        toolbar_clip,
    );
    draw_toolbar_icon_button(
        canvas,
        hitboxes.forward_btn,
        NavIcon::Forward,
        chrome.can_forward,
        toolbar_clip,
    );
    draw_toolbar_icon_button(
        canvas,
        hitboxes.reload_btn,
        NavIcon::Reload,
        true,
        toolbar_clip,
    );
    draw_toolbar_icon_button(
        canvas,
        hitboxes.home_btn,
        NavIcon::Home,
        true,
        toolbar_clip,
    );

    // Centered rounded pill address bar
    let bar = hitboxes.address_bar;
    if bar.width > 8.0 {
        let bar_radius = bar.height * 0.5;
        if chrome.url_focused {
            canvas.fill_rounded_rect_clipped(
                bar,
                bar_radius,
                toolbar_clip,
                Color::rgb(255, 255, 255),
            );
            canvas.draw_rounded_outline_clipped(
                bar,
                bar_radius,
                2.0,
                toolbar_clip,
                Color::rgb(59, 130, 246),
            );
        } else {
            canvas.fill_rounded_rect_clipped(
                bar,
                bar_radius,
                toolbar_clip,
                Color::rgb(241, 245, 249),
            );
        }

        let pad_x = 12.0;
        let inner_w = (bar.width - 2.0 * pad_x).max(0.0);
        let text_clip = Rect {
            x: bar.x + pad_x,
            y: bar.y + 2.0,
            width: inner_w,
            height: (bar.height - 4.0).max(0.0),
        };
        let url_fs = 13.0_f32;
        let text_x = text_clip.x;
        let text_y = bar.y + (bar.height - font::line_height_px(url_fs)) * 0.5;

        let visible_text = if chrome.url_focused {
            tail_slice_to_width(chrome.url_input, url_fs, false, false, (inner_w - 3.0).max(0.0))
                .to_string()
        } else {
            truncate_prefix_to_width(chrome.url_input, url_fs, false, false, inner_w)
        };

        canvas.draw_text_aa(
            font,
            text_x,
            text_y,
            &visible_text,
            Color::rgb(15, 23, 42),
            url_fs,
            false,
            false,
            text_clip,
        );

        if chrome.url_focused {
            let cursor_x = text_x + font::measure_text_px(&visible_text, url_fs, false, false) + 1.0;
            canvas.fill_rect_clipped(
                Rect {
                    x: cursor_x,
                    y: bar.y + 7.0,
                    width: 1.5,
                    height: (bar.height - 14.0).max(0.0),
                },
                text_clip,
                Color::rgb(59, 130, 246),
            );
        }
    }

    // Subtle right-side timing badge pill
    let badge = chrome.timing_badge.trim();
    if !badge.is_empty() && w > 320.0 {
        let badge_fs = 11.0_f32;
        let badge_tw = font::measure_text_px(badge, badge_fs, false, true);
        let avail_right = (w - (bar.x + bar.width + 8.0) - 8.0).max(0.0);
        let pill_w = (badge_tw + 14.0).min(avail_right);
        if pill_w >= 24.0 {
            let pill_h = 22.0_f32;
            let pill = Rect {
                x: w - pill_w - 8.0,
                y: (TAB_BAR_HEIGHT as f32) + ((TOOLBAR_HEIGHT as f32) - pill_h) * 0.5,
                width: pill_w,
                height: pill_h,
            };
            canvas.fill_rounded_rect_clipped(
                pill,
                pill_h * 0.5,
                toolbar_clip,
                Color::rgb(241, 245, 249),
            );
            let txt = truncate_prefix_to_width(badge, badge_fs, false, true, pill_w - 10.0);
            let tx = pill.x + (pill.width - font::measure_text_px(&txt, badge_fs, false, true)) * 0.5;
            let ty = pill.y + (pill.height - font::line_height_px(badge_fs)) * 0.5;
            canvas.draw_text_aa(
                font,
                tx,
                ty,
                &txt,
                Color::rgb(100, 116, 139),
                badge_fs,
                false,
                true,
                pill,
            );
        }
    }
}

fn draw_floating_status_pill(
    canvas: &mut Canvas,
    font: &mut FontEngine,
    viewport_clip: Rect,
    status_text: &str,
) {
    let trimmed = status_text.trim();
    if trimmed.is_empty() || viewport_clip.height < 28.0 || viewport_clip.width < 60.0 {
        return;
    }

    let fs = 12.0_f32;
    let max_pill_w = (viewport_clip.width - 24.0).clamp(0.0, 560.0);
    let max_text_w = (max_pill_w - 18.0).max(0.0);
    let shown = truncate_prefix_to_width(trimmed, fs, false, false, max_text_w);
    if shown.is_empty() {
        return;
    }
    let text_w = font::measure_text_px(&shown, fs, false, false);
    let pill_w = (text_w + 18.0).min(max_pill_w);
    let pill_h = 22.0_f32;
    let pill = Rect {
        x: 10.0,
        y: viewport_clip.y + viewport_clip.height - pill_h - 8.0,
        width: pill_w,
        height: pill_h,
    };

    canvas.fill_rounded_rect_clipped(
        pill,
        6.0,
        viewport_clip,
        Color::rgba(248, 250, 252, 242),
    );
    canvas.draw_rounded_outline_clipped(
        pill,
        6.0,
        1.0,
        viewport_clip,
        Color::rgba(203, 213, 225, 240),
    );

    let text_clip = Rect {
        x: pill.x + 9.0,
        y: pill.y + 2.0,
        width: (pill.width - 18.0).max(0.0),
        height: (pill.height - 4.0).max(0.0),
    };
    let text_y = pill.y + (pill.height - font::line_height_px(fs)) * 0.5;
    canvas.draw_text_aa(
        font,
        text_clip.x,
        text_y,
        &shown,
        Color::rgb(51, 65, 85),
        fs,
        false,
        false,
        text_clip,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Dimensions;
    use crate::style::{ComputedStyle, EdgeSizes};

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

        assert_eq!(canvas.pixels[0], 0x00000000);
        assert_eq!(canvas.pixels[29 * 40 + 39], 0x000A141E);
        assert_eq!(canvas.pixels[15 * 40 + 15], 0x00FFFFFF);

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
        assert_eq!(canvas.pixels[0], 0x00000000);

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
        assert_eq!(canvas.pixels[3], 0x00000000);
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
        assert_eq!(canvas.pixels[0], 0x00FFFFFF);
        // Bottom-right corner outside arc remains white
        assert_eq!(canvas.pixels[19 * 20 + 19], 0x00FFFFFF);
        // Top middle edge & center interior are filled black
        assert_eq!(canvas.pixels[10], 0x00000000);
        assert_eq!(canvas.pixels[10 * 20 + 10], 0x00000000);
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

        assert_eq!(canvas.pixels[6 + 1], 0x00FF0000);
        let blended = canvas.pixels[6 + 2];
        let br = (blended >> 16) & 0xFF;
        let bb = blended & 0xFF;
        assert!((126..=129).contains(&br), "Expected blended red ~127, got {br}");
        assert_eq!(bb, 255);
        assert_eq!(canvas.pixels[2 * 6 + 1], 0x00FFFFFF);
        assert_eq!(canvas.pixels[2 * 6 + 2], 0x000A141E);
    }

    #[test]
    fn ppm_header_and_byte_size() {
        let mut canvas = Canvas::new(3, 2, Color::rgb(1, 2, 3));
        canvas.pixels[0] = 0x00FF8040;
        let ppm = canvas.to_ppm();
        let header = b"P6\n3 2\n255\n";
        assert!(ppm.starts_with(header));
        assert_eq!(ppm.len(), header.len() + 3 * 2 * 3);
        assert_eq!(&ppm[header.len()..header.len() + 3], &[0xFF, 0x80, 0x40]);
    }

    #[test]
    fn render_frame_handles_tiny_canvas_without_underflow() {
        let mut canvas = Canvas::new(10, 10, Color::rgb(255, 255, 255));
        let mut font = FontEngine::new();
        let chrome = ChromeState {
            url_input: "about:droplet",
            url_focused: true,
            can_back: false,
            can_forward: false,
            status_text: "Ready",
            timing_badge: "0.4 ms",
            tab_titles: &["Droplet"],
            active_tab: 0,
            scroll_y: 0.0,
            content_height: 500.0,
        };
        render_frame(&mut canvas, &mut font, &[], &chrome);
    }

    #[test]
    fn build_display_list_emits_bg_borders_and_text() {
        let root = LayoutBox {
            node_id: Some(0),
            dimensions: Dimensions {
                content: Rect {
                    x: 10.0,
                    y: 10.0,
                    width: 100.0,
                    height: 20.0,
                },
                padding: EdgeSizes::default(),
                border: EdgeSizes {
                    top: 1.0,
                    right: 2.0,
                    bottom: 1.0,
                    left: 2.0,
                },
                margin: EdgeSizes::default(),
            },
            box_type: BoxType::Text {
                lines: vec!["Hi".into()],
                line_height: 20.0,
                font_size: 16.0,
                font_scale: 1,
                monospace: false,
            },
            style: ComputedStyle {
                background_color: Some(Color::rgb(240, 240, 240)),
                border_color: Color::rgb(100, 100, 100),
                color: Color::rgb(0, 0, 0),
                bold: true,
                text_underline: true,
                ..Default::default()
            },
            href: None,
            children: vec![],
        };
        let list = build_display_list(&root);
        assert_eq!(list.len(), 6);
    }

    #[test]
    fn build_display_list_emits_rounded_rect_and_image() {
        let img = Arc::new(DecodedImage {
            width: 40,
            height: 20,
            rgba: vec![255; 40 * 20 * 4],
        });
        let root = LayoutBox {
            node_id: Some(1),
            dimensions: Dimensions {
                content: Rect {
                    x: 10.0,
                    y: 10.0,
                    width: 40.0,
                    height: 20.0,
                },
                padding: EdgeSizes::default(),
                border: EdgeSizes::default(),
                margin: EdgeSizes::default(),
            },
            box_type: BoxType::Image {
                image: Some(img),
                alt: "icon".into(),
            },
            style: ComputedStyle {
                background_color: Some(Color::rgb(241, 245, 249)),
                border_radius: 8.0,
                ..Default::default()
            },
            href: None,
            children: vec![],
        };
        let list = build_display_list(&root);
        assert_eq!(list.len(), 2);
        assert!(matches!(
            list[0],
            DisplayCommand::SolidRect { radius, .. } if (radius - 8.0).abs() < 0.01
        ));
        assert!(matches!(list[1], DisplayCommand::Image { .. }));
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
        h_canvas.draw_text_aa(&mut font, 4.0, 4.0, "H", Color::rgb(0, 0, 0), 24.0, false, false, clip);
        let mut dot_canvas = Canvas::new(120, 60, Color::rgb(255, 255, 255));
        dot_canvas.draw_text_aa(&mut font, 4.0, 4.0, ".", Color::rgb(0, 0, 0), 24.0, false, false, clip);

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
