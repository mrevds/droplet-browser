use droplet_gfx::canvas::{blend_alpha, rect_clip_bounds};
use droplet_gfx::font::{self, FontEngine};
use droplet_gfx::{draw_line_aa, Canvas, Color, Rect};

pub const TAB_BAR_HEIGHT: usize = 30;
pub const TOOLBAR_HEIGHT: usize = 44;
pub const CHROME_HEIGHT: usize = TAB_BAR_HEIGHT + TOOLBAR_HEIGHT; // 74
pub const STATUS_BAR_HEIGHT: usize = 0;

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

#[derive(Debug, Clone, Copy)]
pub struct ChromeState<'a> {
    pub url_input: &'a str,
    pub url_focused: bool,
    pub autocomplete_suffix: &'a str,
    pub can_back: bool,
    pub can_forward: bool,
    pub status_text: &'a str,
    pub timing_badge: &'a str,
    pub tab_titles: &'a [&'a str],
    pub active_tab: usize,
    pub scroll_y: f32,
    pub content_height: f32,
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
            if let Some((x0, y0, x1, y1)) =
                rect_clip_bounds(box_r, clip, canvas.width, canvas.height)
            {
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
                        let cov =
                            ((stroke * 0.5 + 0.5 - dist).clamp(0.0, 1.0) * 255.0).round() as u8;
                        if cov > 0 {
                            let idx = row_offset + col;
                            canvas.pixels[idx] = blend_alpha(canvas.pixels[idx], fg, cov);
                        }
                    }
                }
            }
            // Arrow head on top-right of loop
            draw_line_aa(
                canvas,
                cx + 1.0,
                cy - r - 0.2,
                cx + 4.5,
                cy - r + 1.0,
                stroke,
                fg,
                clip,
            );
            draw_line_aa(
                canvas,
                cx + 4.5,
                cy - r + 1.0,
                cx + 3.2,
                cy - r - 2.5,
                stroke,
                fg,
                clip,
            );
        }
        NavIcon::Home => {
            // Roof
            draw_line_aa(canvas, cx - 6.0, cy - 0.5, cx, cy - 6.0, stroke, fg, clip);
            draw_line_aa(canvas, cx, cy - 6.0, cx + 6.0, cy - 0.5, stroke, fg, clip);
            // Walls + base
            draw_line_aa(
                canvas,
                cx - 4.2,
                cy - 1.5,
                cx - 4.2,
                cy + 5.0,
                stroke,
                fg,
                clip,
            );
            draw_line_aa(
                canvas,
                cx + 4.2,
                cy - 1.5,
                cx + 4.2,
                cy + 5.0,
                stroke,
                fg,
                clip,
            );
            draw_line_aa(
                canvas,
                cx - 4.2,
                cy + 5.0,
                cx + 4.2,
                cy + 5.0,
                stroke,
                fg,
                clip,
            );
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

#[allow(dead_code)]
pub(crate) fn draw_chrome(
    canvas: &mut Canvas,
    font: &mut FontEngine,
    chrome_h: usize,
    chrome: &ChromeState<'_>,
) {
    draw_chrome_clipped(
        canvas,
        font,
        chrome_h,
        chrome,
        Rect {
            x: 0.0,
            y: 0.0,
            width: canvas.width as f32,
            height: chrome_h as f32,
        },
    );
}

pub(crate) fn draw_chrome_clipped(
    canvas: &mut Canvas,
    font: &mut FontEngine,
    chrome_h: usize,
    chrome: &ChromeState<'_>,
    damage_clip: Rect,
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
    let chrome_clip = crate::rect_intersect(full_chrome_clip, damage_clip);
    if chrome_clip.width <= 0.0 || chrome_clip.height <= 0.0 {
        return;
    }

    let tab_bar_h = TAB_BAR_HEIGHT.min(chrome_h) as f32;
    let full_tab_clip = Rect {
        x: 0.0,
        y: 0.0,
        width: w,
        height: tab_bar_h,
    };
    let tab_clip = crate::rect_intersect(full_tab_clip, chrome_clip);
    let hitboxes = chrome_hitboxes(canvas.width, chrome.tab_titles.len());

    if tab_clip.width > 0.0 && tab_clip.height > 0.0 {
        // Soft neutral tab strip backdrop
        canvas.fill_rect_clipped(full_tab_clip, tab_clip, Color::rgb(226, 232, 240));

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
            let close_rect = hitboxes
                .tab_close_rects
                .get(idx)
                .copied()
                .unwrap_or_default();
            let right_pad = if close_rect.width > 0.0 { 22.0 } else { 6.0 };
            let title_avail = (tab_rect.width - 10.0 - right_pad).max(0.0);
            let label = truncate_prefix_to_width(title, 12.0, is_active, false, title_avail);
            if !label.is_empty() {
                let text_x = tab_rect.x + 8.0;
                let text_clip = crate::rect_intersect(
                    Rect {
                        x: text_x,
                        y: tab_rect.y + 2.0,
                        width: title_avail,
                        height: (tab_rect.height - 4.0).max(0.0),
                    },
                    tab_clip,
                );
                let ty = tab_rect.y + (tab_rect.height - font::line_height_px(12.0)) * 0.5;
                let fg = if is_active {
                    Color::rgb(15, 23, 42)
                } else {
                    Color::rgb(71, 85, 105)
                };
                canvas.draw_text_aa(
                    font,
                    text_x,
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
    }

    if chrome_h <= TAB_BAR_HEIGHT {
        return;
    }

    // Toolbar backdrop (#ffffff + hairline divider #e2e8f0)
    let full_toolbar_clip = Rect {
        x: 0.0,
        y: TAB_BAR_HEIGHT as f32,
        width: w,
        height: (chrome_h - TAB_BAR_HEIGHT) as f32,
    };
    let toolbar_clip = crate::rect_intersect(full_toolbar_clip, chrome_clip);
    if toolbar_clip.width <= 0.0 || toolbar_clip.height <= 0.0 {
        return;
    }
    canvas.fill_rect_clipped(full_toolbar_clip, toolbar_clip, Color::rgb(255, 255, 255));
    canvas.fill_rect_clipped(
        Rect {
            x: 0.0,
            y: (CHROME_HEIGHT as f32) - 1.0,
            width: w,
            height: 1.0,
        },
        toolbar_clip,
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
        let text_x = bar.x + pad_x;
        let text_clip = crate::rect_intersect(
            Rect {
                x: text_x,
                y: bar.y + 2.0,
                width: inner_w,
                height: (bar.height - 4.0).max(0.0),
            },
            toolbar_clip,
        );
        let url_fs = 13.0_f32;
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

        if chrome.url_focused && !chrome.autocomplete_suffix.is_empty() {
            let typed_w = font::measure_text_px(&visible_text, url_fs, false, false);
            let typed_is_full =
                font::measure_text_px(chrome.url_input, url_fs, false, false) <= inner_w - 3.0;
            if typed_is_full {
                let remaining = (inner_w - typed_w - 3.0).max(0.0);
                let suffix =
                    truncate_prefix_to_width(chrome.autocomplete_suffix, url_fs, false, false, remaining);
                if !suffix.is_empty() {
                    canvas.draw_text_aa(
                        font,
                        text_x + typed_w,
                        text_y,
                        &suffix,
                        Color::rgb(148, 163, 184),
                        url_fs,
                        false,
                        false,
                        text_clip,
                    );
                }
            }
        }

        if chrome.url_focused {
            let cursor_x =
                text_x + font::measure_text_px(&visible_text, url_fs, false, false) + 1.0;
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
                crate::rect_intersect(pill, toolbar_clip),
            );
        }
    }
}

pub(crate) fn draw_floating_status_pill(
    canvas: &mut Canvas,
    font: &mut FontEngine,
    viewport_bounds: Rect,
    draw_clip: Rect,
    status_text: &str,
) {
    let trimmed = status_text.trim();
    if trimmed.is_empty()
        || viewport_bounds.height < 28.0
        || viewport_bounds.width < 60.0
        || draw_clip.width <= 0.0
        || draw_clip.height <= 0.0
    {
        return;
    }

    let fs = 12.0_f32;
    let max_pill_w = (viewport_bounds.width - 24.0).clamp(0.0, 560.0);
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
        y: viewport_bounds.y + viewport_bounds.height - pill_h - 8.0,
        width: pill_w,
        height: pill_h,
    };

    canvas.fill_rounded_rect_clipped(
        pill,
        6.0,
        draw_clip,
        Color::rgba(248, 250, 252, 242),
    );
    canvas.draw_rounded_outline_clipped(
        pill,
        6.0,
        1.0,
        draw_clip,
        Color::rgba(203, 213, 225, 240),
    );

    let text_x = pill.x + 9.0;
    let text_clip = crate::rect_intersect(
        Rect {
            x: text_x,
            y: pill.y + 2.0,
            width: (pill.width - 18.0).max(0.0),
            height: (pill.height - 4.0).max(0.0),
        },
        draw_clip,
    );
    let text_y = pill.y + (pill.height - font::line_height_px(fs)) * 0.5;
    canvas.draw_text_aa(
        font,
        text_x,
        text_y,
        &shown,
        Color::rgb(51, 65, 85),
        fs,
        false,
        false,
        text_clip,
    );
}
