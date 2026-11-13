pub mod chrome;
pub mod display_list;
pub mod forms_overlay;
pub mod tools_overlay;

pub use chrome::{
    chrome_hitboxes, ChromeHitboxes, ChromeState, CHROME_HEIGHT, STATUS_BAR_HEIGHT,
    TAB_BAR_HEIGHT, TOOLBAR_HEIGHT,
};
pub use display_list::{
    build_display_list, build_display_list_with_forms, DisplayCommand, DisplayList,
};
pub use forms_overlay::{
    caret_chars, line_col_at_char, selection_chars, selection_spans, selection_text,
    ControlVisual, FormVisualState, PageSelection, TextEntry, TextPosition,
};
pub use droplet_gfx::{Canvas, Color, DamageRect, Rect};
pub use tools_overlay::{
    find_bar_rect, BoxModelOverlay, FindBarView, FindHighlight, OverlayState, FIND_BAR_HEIGHT,
};

use droplet_gfx::font::{self, FontEngine};

/// Intersects two floating-point screen rectangles, returning an empty rect when disjoint.
pub fn rect_intersect(a: Rect, b: Rect) -> Rect {
    let x1 = a.x.max(b.x);
    let y1 = a.y.max(b.y);
    let x2 = (a.x + a.width).min(b.x + b.width);
    let y2 = (a.y + a.height).min(b.y + b.height);
    Rect {
        x: x1,
        y: y1,
        width: (x2 - x1).max(0.0),
        height: (y2 - y1).max(0.0),
    }
}

/// Returns the damage rectangle covering only the browser chrome strip.
pub fn chrome_damage_rect(width: usize, height: usize) -> DamageRect {
    DamageRect::new(0, 0, width, CHROME_HEIGHT.min(height))
}

/// Returns the damage rectangle covering only the scrolled page viewport.
pub fn viewport_damage_rect(width: usize, height: usize) -> DamageRect {
    let chrome_h = CHROME_HEIGHT.min(height);
    let viewport_h = height
        .saturating_sub(chrome_h)
        .saturating_sub(STATUS_BAR_HEIGHT);
    DamageRect::new(0, chrome_h, width, viewport_h)
}

pub fn render_frame(
    canvas: &mut Canvas,
    font: &mut FontEngine,
    display_list: &[DisplayCommand],
    chrome: &ChromeState<'_>,
) {
    render_frame_ext(canvas, font, display_list, chrome, None);
}

/// Renders the page, chrome and optional find/inspector overlays.
pub fn render_frame_ext(
    canvas: &mut Canvas,
    font: &mut FontEngine,
    display_list: &[DisplayCommand],
    chrome: &ChromeState<'_>,
    overlay: Option<&OverlayState<'_>>,
) {
    let full = DamageRect::full(canvas.width, canvas.height);
    let _ = render_frame_damaged(canvas, font, display_list, chrome, overlay, full);
}

/// Renders only the pixels inside `damage` (clamped to `canvas`) and returns the clamped damage rect.
pub fn render_frame_damaged(
    canvas: &mut Canvas,
    font: &mut FontEngine,
    display_list: &[DisplayCommand],
    chrome: &ChromeState<'_>,
    overlay: Option<&OverlayState<'_>>,
    damage: DamageRect,
) -> DamageRect {
    let clipped = damage.clamp(canvas.width, canvas.height);
    if canvas.width == 0 || canvas.height == 0 || clipped.is_empty() {
        return clipped;
    }

    let damage_clip = clipped.to_rect();
    let w = canvas.width as f32;
    let chrome_h = CHROME_HEIGHT.min(canvas.height);
    let status_h = STATUS_BAR_HEIGHT;
    let viewport_h = canvas.height.saturating_sub(chrome_h).saturating_sub(status_h);

    let viewport_bounds = Rect {
        x: 0.0,
        y: chrome_h as f32,
        width: w,
        height: viewport_h as f32,
    };
    let viewport_clip = rect_intersect(viewport_bounds, damage_clip);

    if viewport_h > 0 && viewport_clip.width > 0.0 && viewport_clip.height > 0.0 {
        canvas.fill_rect_clipped(viewport_bounds, viewport_clip, Color::rgb(255, 255, 255));

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
                    if ty + th + *font_size * 0.5 <= vp_top
                        || ty - *font_size * 0.5 >= vp_bottom
                    {
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

        if let Some(overlay) = overlay {
            tools_overlay::draw_page_overlays(
                canvas,
                font,
                viewport_bounds,
                viewport_clip,
                offset_y,
                overlay,
            );
        }

        // Sleek floating scrollbar thumb
        if chrome.content_height > viewport_h as f32 && canvas.width >= 10 {
            let vh = viewport_h as f32;
            let thumb_h = ((vh / chrome.content_height) * vh).clamp(24.0, vh);
            let max_scroll = (chrome.content_height - vh).max(1.0);
            let ratio = (chrome.scroll_y / max_scroll).clamp(0.0, 1.0);
            let thumb_y = viewport_bounds.y + ratio * (vh - thumb_h);
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

        chrome::draw_floating_status_pill(
            canvas,
            font,
            viewport_bounds,
            viewport_clip,
            chrome.status_text,
        );
    }

    chrome::draw_chrome_clipped(canvas, font, chrome_h, chrome, damage_clip);

    if let Some(overlay) = overlay {
        if let Some(find_bar) = overlay.find_bar {
            tools_overlay::draw_find_bar(
                canvas,
                font,
                chrome_h,
                viewport_bounds,
                viewport_clip,
                find_bar,
            );
        }
    }

    clipped
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use droplet_gfx::font::FontEngine;
    use droplet_gfx::image::DecodedImage;
    use droplet_layout::{BoxType, Dimensions, LayoutBox};
    use droplet_style::{ComputedStyle, EdgeSizes};

    use super::*;

    #[test]
    fn render_frame_handles_tiny_canvas_without_underflow() {
        let mut canvas = Canvas::new(10, 10, Color::rgb(255, 255, 255));
        let mut font = FontEngine::new();
        let chrome = ChromeState {
            url_input: "about:droplet",
            url_focused: true,
            autocomplete_suffix: "",
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
    fn build_display_list_with_forms_emits_focus_ring_caret_and_multi_node_selection() {
        use droplet_layout::{ControlBox, ControlKind, Dimensions};
        use droplet_style::ComputedStyle;

        let text_a = LayoutBox {
            node_id: Some(2),
            dimensions: Dimensions {
                content: Rect {
                    x: 10.0,
                    y: 10.0,
                    width: 80.0,
                    height: 20.0,
                },
                padding: EdgeSizes::default(),
                border: EdgeSizes::default(),
                margin: EdgeSizes::default(),
            },
            box_type: BoxType::Text {
                lines: vec!["Hello".into()],
                line_height: 20.0,
                font_size: 16.0,
                font_scale: 1,
                monospace: false,
            },
            style: ComputedStyle::default(),
            href: None,
            children: vec![],
        };
        let text_b = LayoutBox {
            node_id: Some(3),
            dimensions: Dimensions {
                content: Rect {
                    x: 10.0,
                    y: 34.0,
                    width: 80.0,
                    height: 20.0,
                },
                padding: EdgeSizes::default(),
                border: EdgeSizes::default(),
                margin: EdgeSizes::default(),
            },
            box_type: BoxType::Text {
                lines: vec!["Мир".into()],
                line_height: 20.0,
                font_size: 16.0,
                font_scale: 1,
                monospace: false,
            },
            style: ComputedStyle::default(),
            href: None,
            children: vec![],
        };
        let input_box = LayoutBox {
            node_id: Some(4),
            dimensions: Dimensions {
                content: Rect {
                    x: 10.0,
                    y: 60.0,
                    width: 180.0,
                    height: 20.0,
                },
                padding: EdgeSizes {
                    top: 5.0,
                    right: 8.0,
                    bottom: 5.0,
                    left: 8.0,
                },
                border: EdgeSizes {
                    top: 1.0,
                    right: 1.0,
                    bottom: 1.0,
                    left: 1.0,
                },
                margin: EdgeSizes::default(),
            },
            box_type: BoxType::Control(ControlBox {
                kind: ControlKind::PasswordInput,
                text: "\u{2022}\u{2022}\u{2022}".into(),
                placeholder: "pw".into(),
                checked: false,
                disabled: false,
            }),
            style: ComputedStyle {
                background_color: Some(Color::rgb(255, 255, 255)),
                border_color: Color::rgb(37, 99, 235),
                color: Color::rgb(15, 23, 42),
                font_size: 14.0,
                ..Default::default()
            },
            href: None,
            children: vec![],
        };
        let root = LayoutBox {
            node_id: Some(1),
            dimensions: Dimensions {
                content: Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 300.0,
                    height: 200.0,
                },
                padding: EdgeSizes::default(),
                border: EdgeSizes::default(),
                margin: EdgeSizes::default(),
            },
            box_type: BoxType::Block,
            style: ComputedStyle::default(),
            href: None,
            children: vec![text_a, text_b, input_box],
        };

        let visual = FormVisualState {
            focused: Some(4),
            control: ControlVisual {
                caret_chars: Some(2),
                selection_chars: Some((0, 2)),
            },
            page_selection: Some(PageSelection {
                anchor: TextPosition { node: 2, offset: 1 },
                focus: TextPosition { node: 3, offset: 4 },
            }),
        };
        let list = build_display_list_with_forms(&root, &visual);
        let selection_rects = list
            .iter()
            .filter(|cmd| matches!(cmd, DisplayCommand::SolidRect { color, .. } if color.a == 74))
            .count();
        // Two page text boxes + one control selection rect
        assert_eq!(selection_rects, 3);
        // Focus ring around control (2px wider on each side -> width = 180 + 16 + 2 + 4 = 202)
        assert!(list.iter().any(|cmd| matches!(
            cmd,
            DisplayCommand::SolidRect { rect, color, .. }
                if *color == Color::rgb(37, 99, 235) && (rect.width - 202.0).abs() < 0.1
        )));
        // Caret (1px wide SolidRect)
        assert!(list.iter().any(|cmd| matches!(
            cmd,
            DisplayCommand::SolidRect { rect, color, .. }
                if *color == Color::rgb(15, 23, 42) && (rect.width - 1.0).abs() < 0.01
        )));
        // Password bullets emitted as Text, not plaintext
        assert!(list.iter().any(|cmd| matches!(
            cmd,
            DisplayCommand::Text { text, .. } if text == "\u{2022}\u{2022}\u{2022}"
        )));
    }

    #[test]
    fn render_frame_damaged_preserves_outside_pixels_and_matches_full_inside_rect() {
        let width = 320;
        let height = 220;
        let display_list = vec![
            DisplayCommand::SolidRect {
                rect: Rect {
                    x: 20.0,
                    y: 20.0,
                    width: 200.0,
                    height: 80.0,
                },
                color: Color::rgb(37, 99, 235),
                radius: 6.0,
            },
            DisplayCommand::Text {
                x: 24.0,
                y: 44.0,
                text: "Damaged repaint".to_string(),
                color: Color::rgb(255, 255, 255),
                font_size: 14.0,
                scale: 1,
                bold: true,
                monospace: false,
                underline: false,
                width: 120.0,
            },
        ];
        let chrome = ChromeState {
            tab_titles: &["Tab A", "Tab B"],
            active_tab: 0,
            url_input: "https://example.org",
            autocomplete_suffix: "",
            url_focused: false,
            can_back: true,
            can_forward: false,
            scroll_y: 0.0,
            content_height: 500.0,
            status_text: "Ready",
            timing_badge: "2ms",
        };
        let overlay = OverlayState {
            find_bar: Some(FindBarView {
                query: "Dam",
                current: 1,
                total: 1,
                focused: true,
            }),
            find_highlights: &[FindHighlight {
                rect: Rect {
                    x: 24.0,
                    y: 44.0,
                    width: 40.0,
                    height: 16.0,
                },
                active: true,
            }],
            inspector: None,
        };

        let mut font_full = FontEngine::new();
        let mut full = Canvas::new(width, height, Color::rgb(0, 0, 0));
        render_frame_ext(&mut full, &mut font_full, &display_list, &chrome, Some(&overlay));

        let sentinel = Color::rgb(250, 120, 45);
        let sentinel_u32 = sentinel.to_u32_xrgb();
        let mut font_partial = FontEngine::new();
        let mut partial = Canvas::new(width, height, sentinel);

        let chrome_dmg = chrome_damage_rect(width, height);
        assert_eq!(
            render_frame_damaged(
                &mut partial,
                &mut font_partial,
                &display_list,
                &chrome,
                Some(&overlay),
                chrome_dmg,
            ),
            chrome_dmg
        );
        // Only chrome pixels changed; viewport pixels still carry sentinel.
        for y in 0..height {
            for x in 0..width {
                let idx = y * width + x;
                if y < chrome_dmg.height {
                    assert_eq!(partial.pixels[idx], full.pixels[idx], "chrome mismatch at ({x},{y})");
                } else {
                    assert_eq!(partial.pixels[idx], sentinel_u32, "leaked outside chrome damage at ({x},{y})");
                }
            }
        }

        // Now paint an interior viewport sub-rectangle and verify pixels outside chrome + subrect stay sentinel.
        let sub = DamageRect::new(15, CHROME_HEIGHT + 10, 180, 75);
        let clamped = render_frame_damaged(
            &mut partial,
            &mut font_partial,
            &display_list,
            &chrome,
            Some(&overlay),
            sub,
        );
        assert_eq!(clamped, sub);
        for y in 0..height {
            for x in 0..width {
                let idx = y * width + x;
                let in_chrome = y < chrome_dmg.height;
                let in_sub = x >= sub.x
                    && x < sub.x + sub.width
                    && y >= sub.y
                    && y < sub.y + sub.height;
                if in_chrome || in_sub {
                    assert_eq!(partial.pixels[idx], full.pixels[idx], "pixel mismatch at ({x},{y})");
                } else {
                    assert_eq!(partial.pixels[idx], sentinel_u32, "leaked outside damage at ({x},{y})");
                }
            }
        }

        // Also check viewport_damage_rect produces exact complement below chrome.
        let vp_dmg = viewport_damage_rect(width, height);
        assert_eq!(vp_dmg.y, CHROME_HEIGHT);
        assert_eq!(vp_dmg.height, height - CHROME_HEIGHT);
    }
}
