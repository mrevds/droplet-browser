//! Generic page overlays: find-in-page highlights/bar and the box-model
//! inspector. Rendering lives here so the shell only supplies geometry.

use droplet_gfx::font::{self, FontEngine};
use droplet_gfx::{Canvas, Color, Rect};
use droplet_layout::Dimensions;

use crate::chrome::CHROME_HEIGHT;

pub const FIND_BAR_HEIGHT: f32 = 32.0;
const FIND_BAR_WIDTH: f32 = 360.0;

/// Compact find bar state mirrored into paint.
#[derive(Debug, Clone, Copy)]
pub struct FindBarView<'a> {
    pub query: &'a str,
    pub current: usize,
    pub total: usize,
    pub focused: bool,
}

/// One highlighted match in page coordinates (before chrome offset/scroll).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FindHighlight {
    pub rect: Rect,
    pub active: bool,
}

/// Box-model rings and label for the currently inspected layout box.
#[derive(Debug, Clone, PartialEq)]
pub struct BoxModelOverlay {
    pub margin: Rect,
    pub border: Rect,
    pub padding: Rect,
    pub content: Rect,
    pub label: String,
}

impl BoxModelOverlay {
    pub fn from_dimensions(dimensions: &Dimensions, label: impl Into<String>) -> Self {
        Self {
            margin: dimensions.margin_box(),
            border: dimensions.border_box(),
            padding: dimensions.padding_box(),
            content: dimensions.content,
            label: label.into(),
        }
    }
}

/// All optional overlays composited over the page.
#[derive(Debug, Clone, Copy, Default)]
pub struct OverlayState<'a> {
    pub find_bar: Option<FindBarView<'a>>,
    pub find_highlights: &'a [FindHighlight],
    pub inspector: Option<&'a BoxModelOverlay>,
}

/// Screen-space find bar rect, anchored just below the chrome.
pub fn find_bar_rect(canvas_width: usize, chrome_height: usize) -> Rect {
    let w = canvas_width as f32;
    let width = FIND_BAR_WIDTH.min((w - 24.0).max(0.0));
    Rect {
        x: (w - width - 12.0).max(12.0),
        y: chrome_height as f32 + 8.0,
        width,
        height: FIND_BAR_HEIGHT,
    }
}

pub(crate) fn draw_page_overlays(
    canvas: &mut Canvas,
    font: &mut FontEngine,
    viewport_bounds: Rect,
    draw_clip: Rect,
    offset_y: f32,
    overlay: &OverlayState<'_>,
) {
    if draw_clip.width <= 0.0 || draw_clip.height <= 0.0 {
        return;
    }

    for highlight in overlay.find_highlights {
        let rect = shift(highlight.rect, offset_y);
        let (fill, outline) = if highlight.active {
            (
                Color::rgba(249, 115, 22, 90),
                Color::rgba(234, 88, 12, 235),
            )
        } else {
            (
                Color::rgba(250, 204, 21, 80),
                Color::rgba(202, 138, 4, 170),
            )
        };
        canvas.fill_rect_clipped(rect, draw_clip, fill);
        outline_rect(canvas, rect, 1.0, draw_clip, outline);
    }

    if let Some(inspector) = overlay.inspector {
        let margin = shift(inspector.margin, offset_y);
        let border = shift(inspector.border, offset_y);
        let padding = shift(inspector.padding, offset_y);
        let content = shift(inspector.content, offset_y);

        paint_ring(
            canvas,
            margin,
            draw_clip,
            Color::rgba(239, 68, 68, 36),
            Color::rgba(220, 38, 38, 220),
        );
        paint_ring(
            canvas,
            border,
            draw_clip,
            Color::rgba(245, 158, 11, 52),
            Color::rgba(217, 119, 6, 230),
        );
        paint_ring(
            canvas,
            padding,
            draw_clip,
            Color::rgba(16, 185, 129, 46),
            Color::rgba(5, 150, 105, 230),
        );
        paint_ring(
            canvas,
            content,
            draw_clip,
            Color::rgba(59, 130, 246, 46),
            Color::rgba(37, 99, 235, 235),
        );
        draw_inspector_label(
            canvas,
            font,
            viewport_bounds,
            draw_clip,
            margin,
            &inspector.label,
        );
    }
}

fn paint_ring(canvas: &mut Canvas, rect: Rect, clip: Rect, fill: Color, outline: Color) {
    if rect.width <= 0.0 || rect.height <= 0.0 {
        return;
    }
    canvas.fill_rect_clipped(rect, clip, fill);
    outline_rect(canvas, rect, 1.0, clip, outline);
}

fn outline_rect(canvas: &mut Canvas, rect: Rect, thickness: f32, clip: Rect, color: Color) {
    if rect.width <= 0.0 || rect.height <= 0.0 {
        return;
    }
    let t = thickness.max(0.5);
    canvas.fill_rect_clipped(
        Rect {
            x: rect.x,
            y: rect.y,
            width: rect.width,
            height: t,
        },
        clip,
        color,
    );
    canvas.fill_rect_clipped(
        Rect {
            x: rect.x,
            y: rect.y + rect.height - t,
            width: rect.width,
            height: t,
        },
        clip,
        color,
    );
    canvas.fill_rect_clipped(
        Rect {
            x: rect.x,
            y: rect.y,
            width: t,
            height: rect.height,
        },
        clip,
        color,
    );
    canvas.fill_rect_clipped(
        Rect {
            x: rect.x + rect.width - t,
            y: rect.y,
            width: t,
            height: rect.height,
        },
        clip,
        color,
    );
}

fn draw_inspector_label(
    canvas: &mut Canvas,
    font: &mut FontEngine,
    viewport_bounds: Rect,
    draw_clip: Rect,
    anchor: Rect,
    label: &str,
) {
    if label.is_empty()
        || viewport_bounds.width < 40.0
        || draw_clip.width <= 0.0
        || draw_clip.height <= 0.0
    {
        return;
    }
    let fs = 11.0_f32;
    let max_w = (viewport_bounds.width - 8.0).clamp(40.0, 420.0);
    let text = truncate_prefix(label, fs, max_w - 14.0);
    let text_w = font::measure_text_px(&text, fs, false, true);
    let pill = Rect {
        x: anchor.x.clamp(
            viewport_bounds.x + 2.0,
            (viewport_bounds.x + viewport_bounds.width - text_w - 16.0)
                .max(viewport_bounds.x + 2.0),
        ),
        y: (anchor.y - 19.0).max(viewport_bounds.y + 2.0),
        width: text_w + 14.0,
        height: 17.0,
    };
    canvas.fill_rounded_rect_clipped(pill, 4.0, draw_clip, Color::rgba(15, 23, 42, 235));
    canvas.draw_text_aa(
        font,
        pill.x + 7.0,
        pill.y + 2.0,
        &text,
        Color::rgb(248, 250, 252),
        fs,
        false,
        true,
        crate::rect_intersect(pill, draw_clip),
    );
}

pub(crate) fn draw_find_bar(
    canvas: &mut Canvas,
    font: &mut FontEngine,
    chrome_h: usize,
    viewport_bounds: Rect,
    draw_clip: Rect,
    find: FindBarView<'_>,
) {
    let bar = find_bar_rect(canvas.width, chrome_h.max(CHROME_HEIGHT));
    if bar.width < 80.0
        || bar.y + bar.height > viewport_bounds.y + viewport_bounds.height
        || draw_clip.width <= 0.0
        || draw_clip.height <= 0.0
    {
        return;
    }

    canvas.fill_rounded_rect_clipped(bar, 8.0, draw_clip, Color::rgba(248, 250, 252, 250));
    canvas.draw_rounded_outline_clipped(
        bar,
        8.0,
        1.0,
        draw_clip,
        Color::rgba(148, 163, 184, 230),
    );

    let fs = 12.5_f32;
    let count = format!("{}/{}", find.current.min(find.total), find.total);
    let count_w = font::measure_text_px(&count, 11.0, false, true).max(30.0);
    let text_left = bar.x + 10.0;
    let text_width = (bar.width - count_w - 34.0).max(20.0);
    let text_clip = crate::rect_intersect(
        Rect {
            x: text_left,
            y: bar.y + 2.0,
            width: text_width,
            height: bar.height - 4.0,
        },
        draw_clip,
    );
    let text_y = bar.y + (bar.height - font::line_height_px(fs)) * 0.5;

    if find.query.is_empty() {
        canvas.draw_text_aa(
            font,
            text_left,
            text_y,
            "Find in page",
            Color::rgb(148, 163, 184),
            fs,
            false,
            false,
            text_clip,
        );
    } else {
        let shown = tail_slice(find.query, fs, text_width - 3.0);
        canvas.draw_text_aa(
            font,
            text_left,
            text_y,
            shown,
            Color::rgb(15, 23, 42),
            fs,
            false,
            false,
            text_clip,
        );
        if find.focused {
            let cursor_x = text_left + font::measure_text_px(shown, fs, false, false) + 1.0;
            canvas.fill_rect_clipped(
                Rect {
                    x: cursor_x,
                    y: bar.y + 7.0,
                    width: 1.5,
                    height: bar.height - 14.0,
                },
                text_clip,
                Color::rgb(59, 130, 246),
            );
        }
    }

    let count_color = if find.total == 0 {
        Color::rgb(190, 18, 60)
    } else {
        Color::rgb(71, 85, 105)
    };
    let count_x = bar.x + bar.width - count_w - 10.0;
    let count_clip = crate::rect_intersect(
        Rect {
            x: count_x - 2.0,
            y: bar.y + 2.0,
            width: bar.width - (count_x - bar.x) - 6.0,
            height: bar.height - 4.0,
        },
        draw_clip,
    );
    canvas.draw_text_aa(
        font,
        count_x,
        bar.y + (bar.height - font::line_height_px(11.0)) * 0.5,
        &count,
        count_color,
        11.0,
        false,
        true,
        count_clip,
    );
}

fn shift(rect: Rect, dy: f32) -> Rect {
    Rect {
        x: rect.x,
        y: rect.y + dy,
        width: rect.width,
        height: rect.height,
    }
}

fn truncate_prefix(text: &str, font_size: f32, max_w: f32) -> String {
    if max_w <= 0.0 {
        return String::new();
    }
    if font::measure_text_px(text, font_size, false, true) <= max_w {
        return text.to_string();
    }
    let mut out = String::new();
    for ch in text.chars() {
        let mut next = out.clone();
        next.push(ch);
        if font::measure_text_px(&next, font_size, false, true) > max_w - 12.0 {
            break;
        }
        out = next;
    }
    out.push_str("...");
    out
}

fn tail_slice(text: &str, font_size: f32, max_w: f32) -> &str {
    if max_w <= 0.0 {
        return "";
    }
    if font::measure_text_px(text, font_size, false, false) <= max_w {
        return text;
    }
    for (idx, _) in text.char_indices() {
        let candidate = &text[idx..];
        if font::measure_text_px(candidate, font_size, false, false) <= max_w {
            return candidate;
        }
    }
    ""
}

#[cfg(test)]
mod tests {
    use super::*;
    use droplet_gfx::font::FontEngine;
    use droplet_style::EdgeSizes;

    #[test]
    fn find_bar_rect_stays_inside_narrow_and_wide_windows() {
        let wide = find_bar_rect(1024, CHROME_HEIGHT);
        assert_eq!(wide.width, FIND_BAR_WIDTH);
        assert_eq!(wide.x + wide.width + 12.0, 1024.0);
        assert_eq!(wide.y, CHROME_HEIGHT as f32 + 8.0);

        let narrow = find_bar_rect(300, CHROME_HEIGHT);
        assert_eq!(narrow.width, 276.0);
        assert_eq!(narrow.x, 12.0);

        let tiny = find_bar_rect(10, CHROME_HEIGHT);
        assert_eq!(tiny.width, 0.0);
    }

    #[test]
    fn box_model_overlay_rings_are_nested_and_distinct() {
        let dims = Dimensions {
            content: Rect {
                x: 30.0,
                y: 40.0,
                width: 100.0,
                height: 50.0,
            },
            padding: EdgeSizes {
                top: 5.0,
                right: 6.0,
                bottom: 7.0,
                left: 8.0,
            },
            border: EdgeSizes {
                top: 2.0,
                right: 3.0,
                bottom: 4.0,
                left: 5.0,
            },
            margin: EdgeSizes {
                top: 10.0,
                right: 11.0,
                bottom: 12.0,
                left: 13.0,
            },
        };
        let overlay = BoxModelOverlay::from_dimensions(&dims, "div #7 100×50");
        assert_eq!(overlay.content, dims.content);
        assert_eq!(overlay.padding, dims.padding_box());
        assert_eq!(overlay.border, dims.border_box());
        assert_eq!(overlay.margin, dims.margin_box());
        assert!(overlay.margin.x < overlay.border.x);
        assert!(overlay.border.x < overlay.padding.x);
        assert!(overlay.padding.x < overlay.content.x);
        assert!(overlay.margin.width > overlay.border.width);
        assert_eq!(overlay.label, "div #7 100×50");
    }

    #[test]
    fn overlays_render_without_panicking_on_tiny_canvas() {
        let mut canvas = Canvas::new(18, 18, Color::rgb(255, 255, 255));
        let mut font = FontEngine::new();
        let highlight = FindHighlight {
            rect: Rect {
                x: 2.0,
                y: 2.0,
                width: 6.0,
                height: 4.0,
            },
            active: true,
        };
        let inspector = BoxModelOverlay {
            margin: Rect {
                x: 1.0,
                y: 1.0,
                width: 12.0,
                height: 12.0,
            },
            border: Rect {
                x: 2.0,
                y: 2.0,
                width: 10.0,
                height: 10.0,
            },
            padding: Rect {
                x: 3.0,
                y: 3.0,
                width: 8.0,
                height: 8.0,
            },
            content: Rect {
                x: 4.0,
                y: 4.0,
                width: 6.0,
                height: 6.0,
            },
            label: "div #1 6\u{00d7}6".into(),
        };
        let view = OverlayState {
            find_bar: Some(FindBarView {
                query: "test",
                current: 1,
                total: 3,
                focused: true,
            }),
            find_highlights: &[highlight],
            inspector: Some(&inspector),
        };
        let view_rect = Rect {
            x: 0.0,
            y: 0.0,
            width: 18.0,
            height: 18.0,
        };
        draw_page_overlays(&mut canvas, &mut font, view_rect, view_rect, 0.0, &view);
        let find_rect = Rect {
            x: 0.0,
            y: 10.0,
            width: 18.0,
            height: 8.0,
        };
        draw_find_bar(
            &mut canvas,
            &mut font,
            10,
            find_rect,
            find_rect,
            FindBarView {
                query: "test",
                current: 1,
                total: 3,
                focused: true,
            },
        );
    }

    #[test]
    fn find_highlight_pixels_are_tinted() {
        let mut canvas = Canvas::new(40, 40, Color::rgb(255, 255, 255));
        let mut font = FontEngine::new();
        let highlight = FindHighlight {
            rect: Rect {
                x: 10.0,
                y: 10.0,
                width: 20.0,
                height: 10.0,
            },
            active: false,
        };
        let view = OverlayState {
            find_bar: None,
            find_highlights: &[highlight],
            inspector: None,
        };
        let rect = Rect {
            x: 0.0,
            y: 0.0,
            width: 40.0,
            height: 40.0,
        };
        draw_page_overlays(&mut canvas, &mut font, rect, rect, 0.0, &view);
        assert_ne!(canvas.pixels[15 * 40 + 15], 0x00FF_FFFF);
        assert_eq!(canvas.pixels[5 * 40 + 5], 0x00FF_FFFF);
    }
}
