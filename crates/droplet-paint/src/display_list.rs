use std::sync::Arc;

use droplet_gfx::font;
use droplet_gfx::image::DecodedImage;
use droplet_gfx::{Color, Rect};
use droplet_layout::{BoxType, LayoutBox};
use droplet_style::EdgeSizes;

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

pub fn build_display_list(root: &LayoutBox) -> DisplayList {
    let mut list = Vec::new();
    emit_layout_box(root, &mut list);
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
        if let Some(bg_col) = bg.filter(|c| c.a > 0) {
            list.push(DisplayCommand::SolidRect {
                rect: layout_box.dimensions.padding_box(),
                color: bg_col,
                radius,
            });
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
        BoxType::Image {
            image: Some(img), ..
        } => {
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
