use std::collections::HashMap;
use std::fmt::Write;
use std::sync::Arc;

use droplet_dom::{Document, NodeId};
use droplet_gfx::font::measure_text_px;
use droplet_gfx::image::DecodedImage;
use droplet_gfx::Rect;
use droplet_style::{ComputedStyle, Display, EdgeSizes, StyledNode};

use crate::{block, flex, replaced};

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Dimensions {
    pub content: Rect,
    pub padding: EdgeSizes,
    pub border: EdgeSizes,
    pub margin: EdgeSizes,
}

#[inline]
fn expand_rect(rect: Rect, edge: EdgeSizes) -> Rect {
    Rect {
        x: rect.x - edge.left,
        y: rect.y - edge.top,
        width: rect.width + edge.left + edge.right,
        height: rect.height + edge.top + edge.bottom,
    }
}

impl Dimensions {
    pub fn padding_box(&self) -> Rect {
        expand_rect(self.content, self.padding)
    }

    pub fn border_box(&self) -> Rect {
        expand_rect(self.padding_box(), self.border)
    }

    pub fn margin_box(&self) -> Rect {
        expand_rect(self.border_box(), self.margin)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum BoxType {
    Block,
    Inline,
    InlineBlock,
    Flex,
    AnonymousBlock,
    Text {
        lines: Vec<String>,
        line_height: f32,
        font_size: f32,
        font_scale: usize,
        monospace: bool,
    },
    Image {
        image: Option<Arc<DecodedImage>>,
        alt: String,
    },
}

#[derive(Debug, Clone)]
pub struct LayoutBox {
    pub node_id: Option<NodeId>,
    pub dimensions: Dimensions,
    pub box_type: BoxType,
    pub style: ComputedStyle,
    pub href: Option<String>,
    pub children: Vec<LayoutBox>,
}

pub(crate) struct LayoutCtx<'a> {
    pub(crate) doc: &'a Document,
    pub(crate) images: &'a HashMap<String, Arc<DecodedImage>>,
}

pub fn dump_layout(root: &LayoutBox) -> String {
    let mut out = String::new();
    dump_layout_rec(root, 0, &mut out);
    out
}

fn dump_layout_rec(node: &LayoutBox, depth: usize, out: &mut String) {
    let indent = "  ".repeat(depth);
    let r = node.dimensions.content;
    let kind = match &node.box_type {
        BoxType::Block => "Block",
        BoxType::Inline => "Inline",
        BoxType::InlineBlock => "InlineBlock",
        BoxType::Flex => "Flex",
        BoxType::AnonymousBlock => "AnonymousBlock",
        BoxType::Text { .. } => "Text",
        BoxType::Image { .. } => "Image",
    };
    let _ = write!(
        out,
        "{}{} ({:.1}, {:.1}, {:.1} x {:.1})",
        indent, kind, r.x, r.y, r.width, r.height
    );
    if let Some(ref href) = node.href {
        let _ = write!(out, " href=\"{}\"", href);
    }
    match &node.box_type {
        BoxType::Text { lines, .. } => {
            let joined = lines.join("\\n");
            let preview: String = joined.chars().take(60).collect();
            let ellipsis = if joined.chars().count() > 60 { "..." } else { "" };
            let _ = write!(out, " \"{}{}\"", preview, ellipsis);
        }
        BoxType::Image { image, alt } if image.is_none() && !alt.is_empty() => {
            let _ = write!(out, " alt=\"{}\"", alt);
        }
        _ => {}
    }
    out.push('\n');
    for child in &node.children {
        dump_layout_rec(child, depth + 1, out);
    }
}

pub(crate) fn font_scale_for(font_size: f32) -> usize {
    if font_size >= 22.0 {
        2
    } else {
        1
    }
}

pub(crate) fn measure_text(text: &str, style: ComputedStyle) -> f32 {
    measure_text_px(text, style.font_size, style.bold, style.monospace)
}

pub(crate) fn resolve_width(style: ComputedStyle, containing_width: f32) -> Option<f32> {
    style
        .width
        .or_else(|| style.width_pct.map(|p| (containing_width * (p / 100.0)).max(0.0)))
}

pub(crate) fn resolve_max_width(style: ComputedStyle, containing_width: f32) -> Option<f32> {
    style
        .max_width
        .or_else(|| style.max_width_pct.map(|p| (containing_width * (p / 100.0)).max(0.0)))
}

pub(crate) fn resolve_height(style: ComputedStyle, containing_height: Option<f32>) -> Option<f32> {
    style.height.or_else(|| {
        containing_height.and_then(|ch| style.height_pct.map(|p| (ch * (p / 100.0)).max(0.0)))
    })
}

pub(crate) fn is_block_level(display: Display) -> bool {
    matches!(display, Display::Block | Display::Flex)
}

pub(crate) fn contains_block(node: &StyledNode) -> bool {
    if is_block_level(node.style.display) {
        return true;
    }
    if node.style.display == Display::None || node.style.display == Display::InlineBlock {
        return false;
    }
    node.children.iter().any(contains_block)
}

pub(crate) fn shift_layout_box(root: &mut LayoutBox, dx: f32, dy: f32) {
    if dx.abs() < f32::EPSILON && dy.abs() < f32::EPSILON {
        return;
    }
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        node.dimensions.content.x += dx;
        node.dimensions.content.y += dy;
        for child in &mut node.children {
            stack.push(child);
        }
    }
}

#[inline]
pub(crate) fn fixed_horizontal_margins(style: ComputedStyle) -> (f32, f32) {
    let left = if style.margin_left_auto {
        0.0
    } else {
        style.margin.left
    };
    let right = if style.margin_right_auto {
        0.0
    } else {
        style.margin.right
    };
    (left, right)
}

pub(crate) fn compute_block_horizontal(
    style: ComputedStyle,
    containing_rect: Rect,
    forced_content_w: Option<f32>,
) -> (f32, f32, EdgeSizes) {
    let cw = containing_rect.width;
    let padding = style.padding;
    let border = style.border;
    let mut margin = style.margin;
    let margin_left_auto = style.margin_left_auto;
    let margin_right_auto = style.margin_right_auto;

    let non_content = padding.left + padding.right + border.left + border.right;

    let explicit_w = forced_content_w.or_else(|| resolve_width(style, cw));
    let max_w = if forced_content_w.is_some() {
        None
    } else {
        resolve_max_width(style, cw)
    };

    let (fixed_left, fixed_right) = fixed_horizontal_margins(style);
    let content_w = if let Some(w) = explicit_w {
        let clamped = if let Some(mw) = max_w { w.min(mw) } else { w };
        clamped.max(0.0)
    } else {
        let mut auto_w = (cw - non_content - fixed_left - fixed_right).max(0.0);
        if let Some(mw) = max_w {
            if auto_w > mw {
                auto_w = mw.max(0.0);
            }
        }
        auto_w
    };

    let underflow = cw - non_content - content_w - (fixed_left + fixed_right);

    if explicit_w.is_some() || max_w.is_some() {
        if margin_left_auto && margin_right_auto {
            if underflow > 0.0 {
                margin.left = underflow * 0.5;
                margin.right = underflow * 0.5;
            } else {
                margin.left = 0.0;
                margin.right = 0.0;
            }
        } else if margin_left_auto {
            margin.left = underflow.max(0.0);
        } else if margin_right_auto {
            margin.right = underflow.max(0.0);
        }
    } else {
        if margin_left_auto {
            margin.left = 0.0;
        }
        if margin_right_auto {
            margin.right = 0.0;
        }
    }

    let content_x = containing_rect.x + margin.left + border.left + padding.left;
    (content_x, content_w, margin)
}

pub(crate) fn find_last_baseline_offset_from_margin_top(box_node: &LayoutBox) -> Option<f32> {
    let mb_top = box_node.dimensions.margin_box().y;
    let mut stack = vec![box_node];
    let mut last_baseline: Option<f32> = None;

    while let Some(node) = stack.pop() {
        if let BoxType::Text {
            lines,
            line_height,
            font_size,
            ..
        } = &node.box_type
        {
            let count = lines.len().max(1);
            let last_line_y = node.dimensions.content.y + (count - 1) as f32 * *line_height;
            let y_pad = ((*line_height - *font_size) * 0.5).max(0.0);
            let baseline = last_line_y + y_pad + *font_size * 0.80;
            last_baseline = Some(match last_baseline {
                Some(prev) => prev.max(baseline),
                None => baseline,
            });
        }
        for child in &node.children {
            stack.push(child);
        }
    }

    last_baseline.map(|b| (b - mb_top).max(0.0))
}

pub(crate) fn box_dispatch(
    ctx: &LayoutCtx<'_>,
    styled: &StyledNode,
    containing_rect: Rect,
    border_box_y: f32,
    containing_height: Option<f32>,
    forced_content_w: Option<f32>,
    forced_content_h: Option<f32>,
) -> LayoutBox {
    if replaced::is_img_element(ctx.doc, styled.node_id) {
        return replaced::layout_image_box(
            ctx,
            styled,
            containing_rect,
            border_box_y,
            containing_height,
            forced_content_w,
            forced_content_h,
        );
    }
    match styled.style.display {
        Display::Flex => flex::layout_flex(
            ctx,
            styled,
            containing_rect,
            border_box_y,
            containing_height,
            forced_content_w,
            forced_content_h,
        ),
        Display::Block | Display::InlineBlock | Display::Inline | Display::None => {
            block::layout_block(
                ctx,
                styled,
                containing_rect,
                border_box_y,
                containing_height,
                forced_content_w,
                forced_content_h,
            )
        }
    }
}
