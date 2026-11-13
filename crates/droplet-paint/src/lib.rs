pub mod chrome;
pub mod display_list;

pub use chrome::{
    chrome_hitboxes, ChromeHitboxes, ChromeState, CHROME_HEIGHT, STATUS_BAR_HEIGHT,
    TAB_BAR_HEIGHT, TOOLBAR_HEIGHT,
};
pub use display_list::{build_display_list, DisplayCommand, DisplayList};
pub use droplet_gfx::{Canvas, Color, Rect};

use droplet_gfx::font::{self, FontEngine};

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

        chrome::draw_floating_status_pill(canvas, font, viewport_clip, chrome.status_text);
    }

    chrome::draw_chrome(canvas, font, chrome_h, chrome);
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
}
