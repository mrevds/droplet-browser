use std::collections::HashMap;
use std::fmt::Write;
use std::sync::Arc;

use crate::css::Color;
use crate::dom::{Document, NodeId, NodeKind};
use crate::font;
use crate::image::DecodedImage;
pub use crate::style::EdgeSizes;
use crate::style::{
    AlignItems, ComputedStyle, Display, FlexBasis, FlexDirection, FlexWrap, JustifyContent,
    StyledNode,
};

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub fn expanded_by(&self, edge: EdgeSizes) -> Rect {
        Rect {
            x: self.x - edge.left,
            y: self.y - edge.top,
            width: self.width + edge.left + edge.right,
            height: self.height + edge.top + edge.bottom,
        }
    }

    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px < self.x + self.width && py >= self.y && py < self.y + self.height
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Dimensions {
    pub content: Rect,
    pub padding: EdgeSizes,
    pub border: EdgeSizes,
    pub margin: EdgeSizes,
}

impl Dimensions {
    pub fn padding_box(&self) -> Rect {
        self.content.expanded_by(self.padding)
    }

    pub fn border_box(&self) -> Rect {
        self.padding_box().expanded_by(self.border)
    }

    pub fn margin_box(&self) -> Rect {
        self.border_box().expanded_by(self.margin)
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

struct LayoutCtx<'a> {
    doc: &'a Document,
    images: &'a HashMap<String, Arc<DecodedImage>>,
}

pub fn layout_tree(
    doc: &Document,
    root: &StyledNode,
    viewport_width: f32,
    viewport_height: f32,
    images: &HashMap<String, Arc<DecodedImage>>,
) -> LayoutBox {
    let ctx = LayoutCtx { doc, images };
    let container = Rect {
        x: 0.0,
        y: 0.0,
        width: viewport_width.max(1.0),
        height: viewport_height.max(1.0),
    };
    layout_box_node(&ctx, root, container, 0.0, None, None, None)
}

pub fn hit_test(root: &LayoutBox, x: f32, y: f32) -> Option<&LayoutBox> {
    for child in root.children.iter().rev() {
        if let Some(hit) = hit_test(child, x, y) {
            return Some(hit);
        }
    }
    if root.dimensions.border_box().contains(x, y) {
        Some(root)
    } else {
        None
    }
}

pub fn hit_test_href(root: &LayoutBox, x: f32, y: f32) -> Option<&str> {
    hit_test_href_inner(root, x, y).1
}

fn hit_test_href_inner(node: &LayoutBox, x: f32, y: f32) -> (bool, Option<&str>) {
    for child in node.children.iter().rev() {
        let (hit, href) = hit_test_href_inner(child, x, y);
        if hit {
            return (true, href.or(node.href.as_deref()));
        }
    }
    if node.dimensions.border_box().contains(x, y) {
        (true, node.href.as_deref())
    } else {
        (false, None)
    }
}

pub fn hit_test_node(root: &LayoutBox, x: f32, y: f32) -> Option<NodeId> {
    hit_test_node_inner(root, x, y).1
}

fn hit_test_node_inner(node: &LayoutBox, x: f32, y: f32) -> (bool, Option<NodeId>) {
    for child in node.children.iter().rev() {
        let (hit, id) = hit_test_node_inner(child, x, y);
        if hit {
            return (true, id.or(node.node_id));
        }
    }
    if node.dimensions.border_box().contains(x, y) {
        (true, node.node_id)
    } else {
        (false, None)
    }
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

fn font_scale_for(font_size: f32) -> usize {
    if font_size >= 22.0 {
        2
    } else {
        1
    }
}

fn measure_text(text: &str, style: ComputedStyle) -> f32 {
    font::measure_text_px(text, style.font_size, style.bold, style.monospace)
}

fn resolve_width(style: ComputedStyle, containing_width: f32) -> Option<f32> {
    style
        .width
        .or_else(|| style.width_pct.map(|p| (containing_width * (p / 100.0)).max(0.0)))
}

fn resolve_max_width(style: ComputedStyle, containing_width: f32) -> Option<f32> {
    style
        .max_width
        .or_else(|| style.max_width_pct.map(|p| (containing_width * (p / 100.0)).max(0.0)))
}

fn resolve_height(style: ComputedStyle, containing_height: Option<f32>) -> Option<f32> {
    style.height.or_else(|| {
        containing_height.and_then(|ch| style.height_pct.map(|p| (ch * (p / 100.0)).max(0.0)))
    })
}

fn is_img_element(doc: &Document, node_id: NodeId) -> bool {
    doc.element(node_id)
        .is_some_and(|e| e.tag_name.eq_ignore_ascii_case("img"))
}

fn is_block_level(display: Display) -> bool {
    matches!(display, Display::Block | Display::Flex)
}

fn contains_block(node: &StyledNode) -> bool {
    if is_block_level(node.style.display) {
        return true;
    }
    if node.style.display == Display::None || node.style.display == Display::InlineBlock {
        return false;
    }
    node.children.iter().any(contains_block)
}

fn shift_layout_box(root: &mut LayoutBox, dx: f32, dy: f32) {
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

fn layout_box_node(
    ctx: &LayoutCtx<'_>,
    styled: &StyledNode,
    containing_rect: Rect,
    border_box_y: f32,
    containing_height: Option<f32>,
    forced_content_w: Option<f32>,
    forced_content_h: Option<f32>,
) -> LayoutBox {
    if is_img_element(ctx.doc, styled.node_id) {
        return layout_image(
            ctx,
            styled,
            containing_rect,
            border_box_y,
            containing_height,
            forced_content_w,
            forced_content_h,
        );
    }
    if styled.style.display == Display::Flex {
        return layout_flex(
            ctx,
            styled,
            containing_rect,
            border_box_y,
            containing_height,
            forced_content_w,
            forced_content_h,
        );
    }
    layout_block(
        ctx,
        styled,
        containing_rect,
        border_box_y,
        containing_height,
        forced_content_w,
        forced_content_h,
    )
}

fn parse_html_dim_attr(raw: &str) -> Option<f32> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.ends_with('%') {
        return None;
    }
    let num_str = trimmed
        .strip_suffix("px")
        .or_else(|| trimmed.strip_suffix("PX"))
        .unwrap_or(trimmed)
        .trim();
    num_str
        .parse::<f32>()
        .ok()
        .filter(|v| v.is_finite() && *v >= 0.0)
}

fn lookup_image(ctx: &LayoutCtx<'_>, src: Option<&str>) -> Option<Arc<DecodedImage>> {
    let raw = src?;
    ctx.images
        .get(raw)
        .or_else(|| ctx.images.get(raw.trim()))
        .cloned()
}

fn resolve_image_content_size(
    ctx: &LayoutCtx<'_>,
    styled: &StyledNode,
    containing_width: f32,
    containing_height: Option<f32>,
    forced_content_w: Option<f32>,
    forced_content_h: Option<f32>,
) -> (Option<Arc<DecodedImage>>, String, f32, f32, bool) {
    let elem = ctx.doc.element(styled.node_id);
    let src = elem.and_then(|e| e.attr("src"));
    let alt = elem.and_then(|e| e.attr("alt")).unwrap_or("").to_string();
    let decoded = lookup_image(ctx, src);

    let css_w = resolve_width(styled.style, containing_width);
    let css_h = resolve_height(styled.style, containing_height);
    let max_w = resolve_max_width(styled.style, containing_width);
    let attr_w = elem.and_then(|e| e.attr("width")).and_then(parse_html_dim_attr);
    let attr_h = elem
        .and_then(|e| e.attr("height"))
        .and_then(parse_html_dim_attr);

    let given_w = forced_content_w.or(css_w).or(attr_w);
    let given_h = forced_content_h.or(css_h).or(attr_h);

    if let Some(ref img) = decoded {
        let iw = img.width as f32;
        let ih = img.height as f32;
        let (mut w, mut h) = if iw > 0.0 && ih > 0.0 {
            match (given_w, given_h) {
                (Some(w), Some(h)) => (w.max(0.0), h.max(0.0)),
                (Some(w), None) => {
                    let w = w.max(0.0);
                    (w, (w * ih / iw).max(0.0))
                }
                (None, Some(h)) => {
                    let h = h.max(0.0);
                    ((h * iw / ih).max(0.0), h)
                }
                (None, None) => (iw, ih),
            }
        } else {
            (given_w.unwrap_or(0.0).max(0.0), given_h.unwrap_or(0.0).max(0.0))
        };

        if let Some(mw) = max_w {
            if w > mw {
                w = mw.max(0.0);
                if given_h.is_none() && iw > 0.0 && ih > 0.0 {
                    h = (w * ih / iw).max(0.0);
                }
            }
        }
        (decoded, alt, w, h, false)
    } else {
        let label = if alt.is_empty() { "[img]" } else { alt.as_str() };
        let text_w = measure_text(label, styled.style).max(24.0);
        let text_h = font::line_height_px(styled.style.font_size).max(20.0);
        let mut w = given_w.unwrap_or(text_w).max(0.0);
        if let Some(mw) = max_w {
            w = w.min(mw).max(0.0);
        }
        let h = given_h.unwrap_or(text_h).max(0.0);
        let needs_default_chrome = given_w.is_none() && given_h.is_none();
        (None, alt, w, h, needs_default_chrome)
    }
}

fn layout_image(
    ctx: &LayoutCtx<'_>,
    styled: &StyledNode,
    containing_rect: Rect,
    border_box_y: f32,
    containing_height: Option<f32>,
    forced_content_w: Option<f32>,
    forced_content_h: Option<f32>,
) -> LayoutBox {
    let (decoded, alt, content_w, content_h, needs_default_chrome) = resolve_image_content_size(
        ctx,
        styled,
        containing_rect.width,
        containing_height,
        forced_content_w,
        forced_content_h,
    );

    let mut padding = styled.style.padding;
    let mut border = styled.style.border;
    if decoded.is_none() {
        if needs_default_chrome && padding == EdgeSizes::default() {
            padding = EdgeSizes {
                top: 4.0,
                right: 6.0,
                bottom: 4.0,
                left: 6.0,
            };
        }
        if border == EdgeSizes::default() {
            border = EdgeSizes {
                top: 1.0,
                right: 1.0,
                bottom: 1.0,
                left: 1.0,
            };
        }
    }

    let mut margin = styled.style.margin;
    let non_content = padding.left + padding.right + border.left + border.right;
    if styled.style.display == Display::Block {
        let used_fixed = (if styled.style.margin_left_auto {
            0.0
        } else {
            margin.left
        }) + (if styled.style.margin_right_auto {
            0.0
        } else {
            margin.right
        });
        let underflow = containing_rect.width - non_content - content_w - used_fixed;
        if styled.style.margin_left_auto && styled.style.margin_right_auto {
            let side = (underflow * 0.5).max(0.0);
            margin.left = side;
            margin.right = side;
        } else if styled.style.margin_left_auto {
            margin.left = underflow.max(0.0);
        } else if styled.style.margin_right_auto {
            margin.right = underflow.max(0.0);
        }
    } else {
        if styled.style.margin_left_auto {
            margin.left = 0.0;
        }
        if styled.style.margin_right_auto {
            margin.right = 0.0;
        }
    }

    let content_x = containing_rect.x + margin.left + border.left + padding.left;
    let content_y = border_box_y + border.top + padding.top;

    LayoutBox {
        node_id: Some(styled.node_id),
        dimensions: Dimensions {
            content: Rect {
                x: content_x,
                y: content_y,
                width: content_w,
                height: content_h,
            },
            padding,
            border,
            margin,
        },
        box_type: BoxType::Image {
            image: decoded,
            alt,
        },
        style: styled.style,
        href: styled.href.clone(),
        children: Vec::new(),
    }
}

fn compute_block_horizontal(
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

    let content_w = if let Some(w) = explicit_w {
        let clamped = if let Some(mw) = max_w { w.min(mw) } else { w };
        clamped.max(0.0)
    } else {
        let fixed_left = if margin_left_auto { 0.0 } else { margin.left };
        let fixed_right = if margin_right_auto { 0.0 } else { margin.right };
        let mut auto_w = (cw - non_content - fixed_left - fixed_right).max(0.0);
        if let Some(mw) = max_w {
            if auto_w > mw {
                auto_w = mw.max(0.0);
            }
        }
        auto_w
    };

    let used_fixed_margins = (if margin_left_auto { 0.0 } else { margin.left })
        + (if margin_right_auto { 0.0 } else { margin.right });
    let underflow = cw - non_content - content_w - used_fixed_margins;

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

fn collapse_margins(a: f32, b: f32) -> f32 {
    if a >= 0.0 && b >= 0.0 {
        a.max(b)
    } else if a < 0.0 && b < 0.0 {
        a.min(b)
    } else {
        a + b
    }
}

fn list_marker_text(doc: &Document, node_id: NodeId) -> Option<String> {
    let elem = doc.element(node_id)?;
    if !elem.tag_name.eq_ignore_ascii_case("li") {
        return None;
    }
    let parent_id = doc.get(node_id)?.parent?;
    let parent_elem = doc.element(parent_id)?;
    if parent_elem.tag_name.eq_ignore_ascii_case("ul") {
        return Some("• ".to_string());
    }
    if parent_elem.tag_name.eq_ignore_ascii_case("ol") {
        let mut idx = 1usize;
        for &cid in &doc.get(parent_id)?.children {
            if cid == node_id {
                return Some(format!("{}. ", idx));
            }
            if doc
                .element(cid)
                .is_some_and(|e| e.tag_name.eq_ignore_ascii_case("li"))
            {
                idx += 1;
            }
        }
    }
    None
}

fn layout_block(
    ctx: &LayoutCtx<'_>,
    styled: &StyledNode,
    containing_rect: Rect,
    border_box_y: f32,
    containing_height: Option<f32>,
    forced_content_w: Option<f32>,
    forced_content_h: Option<f32>,
) -> LayoutBox {
    let (content_x, content_w, margin) =
        compute_block_horizontal(styled.style, containing_rect, forced_content_w);
    let padding = styled.style.padding;
    let border = styled.style.border;
    let content_y = border_box_y + border.top + padding.top;

    let explicit_h = forced_content_h.or_else(|| resolve_height(styled.style, containing_height));

    let child_container = Rect {
        x: content_x,
        y: content_y,
        width: content_w,
        height: explicit_h.unwrap_or(0.0),
    };

    let visible_children: Vec<&StyledNode> = styled
        .children
        .iter()
        .filter(|c| c.style.display != Display::None)
        .collect();

    let has_any_block = visible_children.iter().any(|c| contains_block(c));
    let mut layout_children: Vec<LayoutBox> = Vec::new();
    let mut cursor_y = content_y;
    let mut prev_margin_bottom = 0.0_f32;

    if !has_any_block {
        if !visible_children.is_empty() {
            let (inline_boxes, inline_height) = layout_inline_sequence(
                ctx,
                &visible_children,
                content_x,
                cursor_y,
                content_w,
                explicit_h,
            );
            layout_children.extend(inline_boxes);
            cursor_y += inline_height;
        }
    } else {
        let mut inline_run: Vec<&StyledNode> = Vec::new();
        for child in visible_children {
            if !contains_block(child) {
                inline_run.push(child);
            } else {
                if !inline_run.is_empty() {
                    cursor_y += prev_margin_bottom;
                    prev_margin_bottom = 0.0;
                    let anon = layout_anonymous_block(
                        ctx,
                        &inline_run,
                        styled.style,
                        content_x,
                        cursor_y,
                        content_w,
                        explicit_h,
                    );
                    let anon_h = anon.dimensions.border_box().height;
                    cursor_y += anon_h;
                    layout_children.push(anon);
                    inline_run.clear();
                }

                let child_boxes = flatten_blocks_if_inline_wrapper(child);
                for block_node in child_boxes {
                    let gap = collapse_margins(prev_margin_bottom, block_node.style.margin.top);
                    let child_bb_y = cursor_y + gap;
                    let child_layout = layout_box_node(
                        ctx,
                        block_node,
                        child_container,
                        child_bb_y,
                        explicit_h,
                        None,
                        None,
                    );
                    let bb = child_layout.dimensions.border_box();
                    cursor_y = bb.y + bb.height;
                    prev_margin_bottom = child_layout.dimensions.margin.bottom;
                    layout_children.push(child_layout);
                }
            }
        }
        if !inline_run.is_empty() {
            cursor_y += prev_margin_bottom;
            prev_margin_bottom = 0.0;
            let anon = layout_anonymous_block(
                ctx,
                &inline_run,
                styled.style,
                content_x,
                cursor_y,
                content_w,
                explicit_h,
            );
            let anon_h = anon.dimensions.border_box().height;
            cursor_y += anon_h;
            layout_children.push(anon);
        }
    }

    if let Some(marker) = list_marker_text(ctx.doc, styled.node_id) {
        let fs = styled.style.font_size;
        let lh = font::line_height_px(fs);
        let marker_w = measure_text(&marker, styled.style);
        if content_x >= marker_w {
            let first_y = find_first_text_y(&layout_children).unwrap_or(content_y);
            let mut marker_style = styled.style;
            marker_style.background_color = None;
            marker_style.padding = EdgeSizes::default();
            marker_style.border = EdgeSizes::default();
            marker_style.text_underline = false;
            layout_children.insert(
                0,
                LayoutBox {
                    node_id: None,
                    dimensions: Dimensions {
                        content: Rect {
                            x: content_x - marker_w,
                            y: first_y,
                            width: marker_w,
                            height: lh,
                        },
                        ..Dimensions::default()
                    },
                    box_type: BoxType::Text {
                        lines: vec![marker],
                        line_height: lh,
                        font_size: fs,
                        font_scale: font_scale_for(fs),
                        monospace: styled.style.monospace,
                    },
                    style: marker_style,
                    href: None,
                    children: Vec::new(),
                },
            );
        }
    }

    let inner_height = (cursor_y + prev_margin_bottom - content_y).max(0.0);
    let content_h = match explicit_h {
        Some(h) => h.max(inner_height),
        None => inner_height,
    };

    let box_type = match styled.style.display {
        Display::InlineBlock => BoxType::InlineBlock,
        _ => BoxType::Block,
    };

    LayoutBox {
        node_id: Some(styled.node_id),
        dimensions: Dimensions {
            content: Rect {
                x: content_x,
                y: content_y,
                width: content_w,
                height: content_h,
            },
            padding,
            border,
            margin,
        },
        box_type,
        style: styled.style,
        href: styled.href.clone(),
        children: layout_children,
    }
}

fn find_first_text_y(boxes: &[LayoutBox]) -> Option<f32> {
    let mut stack: Vec<&LayoutBox> = boxes.iter().rev().collect();
    while let Some(b) = stack.pop() {
        if matches!(b.box_type, BoxType::Text { .. }) {
            return Some(b.dimensions.content.y);
        }
        for c in b.children.iter().rev() {
            stack.push(c);
        }
    }
    None
}

fn flatten_blocks_if_inline_wrapper(node: &StyledNode) -> Vec<&StyledNode> {
    if is_block_level(node.style.display) {
        return vec![node];
    }
    let mut out = Vec::new();
    for child in &node.children {
        if child.style.display != Display::None {
            if contains_block(child) {
                out.extend(flatten_blocks_if_inline_wrapper(child));
            } else {
                out.push(child);
            }
        }
    }
    out
}

fn layout_anonymous_block(
    ctx: &LayoutCtx<'_>,
    inline_nodes: &[&StyledNode],
    parent_style: ComputedStyle,
    x: f32,
    y: f32,
    width: f32,
    containing_height: Option<f32>,
) -> LayoutBox {
    let (children, height) =
        layout_inline_sequence(ctx, inline_nodes, x, y, width, containing_height);
    let mut anon_style = parent_style;
    anon_style.background_color = None;
    anon_style.padding = EdgeSizes::default();
    anon_style.border = EdgeSizes::default();
    anon_style.margin = EdgeSizes::default();
    anon_style.border_radius = 0.0;
    LayoutBox {
        node_id: None,
        dimensions: Dimensions {
            content: Rect {
                x,
                y,
                width,
                height,
            },
            ..Dimensions::default()
        },
        box_type: BoxType::AnonymousBlock,
        style: anon_style,
        href: None,
        children,
    }
}

fn layout_inline_block_atomic(
    ctx: &LayoutCtx<'_>,
    styled: &StyledNode,
    avail_w: f32,
    containing_height: Option<f32>,
) -> LayoutBox {
    if is_img_element(ctx.doc, styled.node_id) {
        let container = Rect {
            x: 0.0,
            y: 0.0,
            width: avail_w.max(0.0),
            height: containing_height.unwrap_or(0.0),
        };
        return layout_image(ctx, styled, container, 0.0, containing_height, None, None);
    }

    let pad = styled.style.padding;
    let bdr = styled.style.border;
    let margin = styled.style.margin;
    let margin_h = (if styled.style.margin_left_auto {
        0.0
    } else {
        margin.left
    }) + (if styled.style.margin_right_auto {
        0.0
    } else {
        margin.right
    });
    let non_content = pad.left + pad.right + bdr.left + bdr.right;
    let avail_content_w = (avail_w - margin_h - non_content).max(0.0);

    let explicit_w = resolve_width(styled.style, avail_w);
    let max_w = resolve_max_width(styled.style, avail_w);
    let content_w = if let Some(w) = explicit_w {
        let clamped = if let Some(mw) = max_w { w.min(mw) } else { w };
        clamped.max(0.0)
    } else {
        let (min_w, max_intrinsic) =
            measure_intrinsic_content_width(ctx, styled, avail_content_w);
        let mut stf = max_intrinsic
            .min(avail_content_w)
            .max(min_w.min(avail_content_w));
        if let Some(mw) = max_w {
            stf = stf.min(mw);
        }
        stf.max(0.0)
    };

    let container = Rect {
        x: 0.0,
        y: 0.0,
        width: content_w + non_content + margin_h,
        height: containing_height.unwrap_or(0.0),
    };
    let mut laid = if styled.style.display == Display::Flex {
        layout_flex(
            ctx,
            styled,
            container,
            0.0,
            containing_height,
            Some(content_w),
            None,
        )
    } else {
        layout_block(
            ctx,
            styled,
            container,
            0.0,
            containing_height,
            Some(content_w),
            None,
        )
    };
    if laid.style.margin_left_auto {
        laid.dimensions.margin.left = 0.0;
    }
    if laid.style.margin_right_auto {
        laid.dimensions.margin.right = 0.0;
    }
    if styled.style.display == Display::InlineBlock {
        laid.box_type = BoxType::InlineBlock;
    }
    laid
}

fn measure_intrinsic_content_width(
    ctx: &LayoutCtx<'_>,
    node: &StyledNode,
    avail_w: f32,
) -> (f32, f32) {
    if node.style.display == Display::None {
        return (0.0, 0.0);
    }

    if is_img_element(ctx.doc, node.node_id) {
        let (_, _, w, _, _) = resolve_image_content_size(ctx, node, avail_w, None, None, None);
        return (w, w);
    }

    match ctx.doc.get(node.node_id).map(|n| &n.kind) {
        Some(NodeKind::Text(text)) => {
            if node.style.preformatted {
                let norm = normalize_preformatted(text);
                let mut max_w = 0.0_f32;
                for line in norm.split('\n') {
                    max_w = max_w.max(measure_text(line, node.style));
                }
                (max_w, max_w)
            } else {
                let collapsed = collapse_html_whitespace(text);
                let max_w = measure_text(collapsed.trim(), node.style);
                let mut min_w = 0.0_f32;
                for word in collapsed.split_whitespace() {
                    min_w = min_w.max(measure_text(word, node.style));
                }
                (min_w, max_w)
            }
        }
        Some(NodeKind::Element(elem)) => {
            if elem.tag_name.eq_ignore_ascii_case("br") {
                return (0.0, 0.0);
            }
            let explicit_w = resolve_width(node.style, avail_w);
            let max_w = resolve_max_width(node.style, avail_w);
            if let Some(w) = explicit_w {
                let clamped = if let Some(mw) = max_w { w.min(mw) } else { w }.max(0.0);
                return (clamped, clamped);
            }

            let child_pad_border = node.style.padding.left
                + node.style.padding.right
                + node.style.border.left
                + node.style.border.right;
            let child_avail = (avail_w - child_pad_border).max(0.0);

            if node.style.display == Display::Flex {
                let is_row = matches!(
                    node.style.flex_direction,
                    FlexDirection::Row | FlexDirection::RowReverse
                );
                let gap = node.style.gap.max(0.0);
                let mut min_acc = 0.0_f32;
                let mut max_acc = 0.0_f32;
                let mut count = 0usize;

                for child in &node.children {
                    if is_skipped_flex_child(ctx.doc, child) {
                        continue;
                    }
                    let (c_min, c_max) = measure_intrinsic_content_width(ctx, child, child_avail);
                    let extra = child.style.padding.left
                        + child.style.padding.right
                        + child.style.border.left
                        + child.style.border.right
                        + (if child.style.margin_left_auto {
                            0.0
                        } else {
                            child.style.margin.left
                        })
                        + (if child.style.margin_right_auto {
                            0.0
                        } else {
                            child.style.margin.right
                        });
                    let basis_w = match child.style.flex_basis {
                        FlexBasis::Px(px) if is_row => Some(px.max(0.0)),
                        _ => None,
                    };
                    let o_min = basis_w.unwrap_or(c_min) + extra;
                    let o_max = basis_w.unwrap_or(c_max) + extra;
                    if is_row {
                        max_acc += o_max;
                        if node.style.flex_wrap == FlexWrap::NoWrap {
                            min_acc += o_min;
                        } else {
                            min_acc = min_acc.max(o_min);
                        }
                    } else {
                        min_acc = min_acc.max(o_min);
                        max_acc = max_acc.max(o_max);
                    }
                    count += 1;
                }
                if is_row && count > 1 {
                    let g = gap * (count - 1) as f32;
                    max_acc += g;
                    if node.style.flex_wrap == FlexWrap::NoWrap {
                        min_acc += g;
                    }
                }
                if let Some(mw) = max_w {
                    min_acc = min_acc.min(mw);
                    max_acc = max_acc.min(mw);
                }
                return (min_acc.max(0.0), max_acc.max(0.0));
            }

            let mut min_res = 0.0_f32;
            let mut max_res = 0.0_f32;
            let mut cur_inline_max = 0.0_f32;

            for child in &node.children {
                if child.style.display == Display::None {
                    continue;
                }
                let is_br = ctx
                    .doc
                    .element(child.node_id)
                    .is_some_and(|e| e.tag_name.eq_ignore_ascii_case("br"));
                if is_br {
                    max_res = max_res.max(cur_inline_max);
                    cur_inline_max = 0.0;
                    continue;
                }

                let (c_min, c_max) = measure_intrinsic_content_width(ctx, child, child_avail);
                let pad_bdr = child.style.padding.left
                    + child.style.padding.right
                    + child.style.border.left
                    + child.style.border.right;
                let mar = if child.style.display == Display::Inline {
                    0.0
                } else {
                    (if child.style.margin_left_auto {
                        0.0
                    } else {
                        child.style.margin.left
                    }) + (if child.style.margin_right_auto {
                        0.0
                    } else {
                        child.style.margin.right
                    })
                };

                if contains_block(child) {
                    max_res = max_res.max(cur_inline_max);
                    cur_inline_max = 0.0;
                    min_res = min_res.max(c_min + pad_bdr + mar);
                    max_res = max_res.max(c_max + pad_bdr + mar);
                } else {
                    min_res = min_res.max(c_min + pad_bdr + mar);
                    cur_inline_max += c_max + pad_bdr + mar;
                }
            }
            max_res = max_res.max(cur_inline_max);
            if let Some(mw) = max_w {
                min_res = min_res.min(mw);
                max_res = max_res.min(mw);
            }
            (min_res.max(0.0), max_res.max(0.0))
        }
        _ => {
            let mut min_res = 0.0_f32;
            let mut max_res = 0.0_f32;
            for child in &node.children {
                if child.style.display == Display::None {
                    continue;
                }
                let (c_min, c_max) = measure_intrinsic_content_width(ctx, child, avail_w);
                min_res = min_res.max(c_min);
                max_res = max_res.max(c_max);
            }
            (min_res, max_res)
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct InlineDecor {
    span_id: Option<usize>,
    background_color: Option<Color>,
    border_color: Color,
    border_radius: f32,
    padding: EdgeSizes,
    border: EdgeSizes,
}

enum InlinePiece<'a> {
    LineBreak {
        line_height: f32,
    },
    AtomicBox {
        node: &'a StyledNode,
    },
    Text {
        text: String,
        node_id: NodeId,
        style: Box<ComputedStyle>,
        href: Option<String>,
        span_id: Option<usize>,
        left_pad: f32,
        left_border: f32,
        right_pad: f32,
        right_border: f32,
    },
}

enum InlineToken<'a> {
    LineBreak {
        line_height: f32,
    },
    AtomicBox {
        node: &'a StyledNode,
    },
    Space {
        node_id: NodeId,
        style: ComputedStyle,
        href: Option<String>,
        span_id: Option<usize>,
        left_pad: f32,
        left_border: f32,
        right_pad: f32,
        right_border: f32,
    },
    Word {
        text: String,
        node_id: NodeId,
        style: ComputedStyle,
        href: Option<String>,
        span_id: Option<usize>,
        left_pad: f32,
        left_border: f32,
        right_pad: f32,
        right_border: f32,
    },
    PreSegment {
        text: String,
        newline_before: bool,
        node_id: NodeId,
        style: ComputedStyle,
        href: Option<String>,
        span_id: Option<usize>,
        left_pad: f32,
        left_border: f32,
        right_pad: f32,
        right_border: f32,
    },
}

struct TextLineFragment {
    text: String,
    node_id: Option<NodeId>,
    style: ComputedStyle,
    href: Option<String>,
    span_id: Option<usize>,
    line_height: f32,
    text_width: f32,
    left_pad: f32,
    left_border: f32,
    right_pad: f32,
    right_border: f32,
}

enum LineItem {
    Text(TextLineFragment),
    Atomic(LayoutBox),
}

fn collect_inline_pieces<'a>(
    doc: &Document,
    node: &'a StyledNode,
    decor: &InlineDecor,
    next_span_uid: &mut usize,
    out: &mut Vec<InlinePiece<'a>>,
) {
    if node.style.display == Display::None {
        return;
    }
    if node.style.display == Display::InlineBlock || is_img_element(doc, node.node_id) {
        out.push(InlinePiece::AtomicBox { node });
        return;
    }
    match doc.get(node.node_id).map(|n| &n.kind) {
        Some(NodeKind::Text(text)) => {
            if text.is_empty() {
                return;
            }
            let mut effective = node.style;
            effective.background_color = decor.background_color;
            effective.border_color = decor.border_color;
            effective.border_radius = decor.border_radius;
            effective.padding = EdgeSizes {
                top: decor.padding.top,
                bottom: decor.padding.bottom,
                left: 0.0,
                right: 0.0,
            };
            effective.border = EdgeSizes {
                top: decor.border.top,
                bottom: decor.border.bottom,
                left: 0.0,
                right: 0.0,
            };

            out.push(InlinePiece::Text {
                text: text.clone(),
                node_id: node.node_id,
                style: Box::new(effective),
                href: node.href.clone(),
                span_id: decor.span_id,
                left_pad: decor.padding.left,
                left_border: decor.border.left,
                right_pad: decor.padding.right,
                right_border: decor.border.right,
            });
        }
        Some(NodeKind::Element(elem)) => {
            if elem.tag_name.eq_ignore_ascii_case("br") {
                out.push(InlinePiece::LineBreak {
                    line_height: font::line_height_px(node.style.font_size),
                });
                return;
            }

            let has_own_box_decor = node.style.background_color.is_some()
                || node.style.padding != EdgeSizes::default()
                || node.style.border != EdgeSizes::default();

            let child_decor = if has_own_box_decor {
                *next_span_uid += 1;
                InlineDecor {
                    span_id: Some(*next_span_uid),
                    background_color: node.style.background_color.or(decor.background_color),
                    border_color: if node.style.border != EdgeSizes::default() {
                        node.style.border_color
                    } else {
                        decor.border_color
                    },
                    border_radius: node.style.border_radius.max(decor.border_radius),
                    padding: node.style.padding,
                    border: node.style.border,
                }
            } else {
                *decor
            };

            let start_idx = out.len();
            for child in &node.children {
                collect_inline_pieces(doc, child, &child_decor, next_span_uid, out);
            }

            if has_own_box_decor && out.len() > start_idx {
                let target_id = child_decor.span_id;
                let mut seen_first = false;
                for piece in out[start_idx..].iter_mut() {
                    if let InlinePiece::Text {
                        span_id,
                        left_pad,
                        left_border,
                        ..
                    } = piece
                    {
                        if *span_id == target_id {
                            if seen_first {
                                *left_pad = 0.0;
                                *left_border = 0.0;
                            } else {
                                seen_first = true;
                            }
                        }
                    }
                }
                let mut seen_last = false;
                for piece in out[start_idx..].iter_mut().rev() {
                    if let InlinePiece::Text {
                        span_id,
                        right_pad,
                        right_border,
                        ..
                    } = piece
                    {
                        if *span_id == target_id {
                            if seen_last {
                                *right_pad = 0.0;
                                *right_border = 0.0;
                            } else {
                                seen_last = true;
                            }
                        }
                    }
                }
            }
        }
        _ => {
            for child in &node.children {
                collect_inline_pieces(doc, child, decor, next_span_uid, out);
            }
        }
    }
}

fn tokenize_pieces<'a>(pieces: Vec<InlinePiece<'a>>) -> Vec<InlineToken<'a>> {
    let mut tokens = Vec::new();
    let total_pieces = pieces.len();

    for (piece_idx, piece) in pieces.into_iter().enumerate() {
        match piece {
            InlinePiece::LineBreak { line_height } => {
                tokens.push(InlineToken::LineBreak { line_height });
            }
            InlinePiece::AtomicBox { node } => {
                tokens.push(InlineToken::AtomicBox { node });
            }
            InlinePiece::Text {
                text,
                node_id,
                style,
                href,
                span_id,
                left_pad,
                left_border,
                right_pad,
                right_border,
            } => {
                let style = *style;
                if style.preformatted {
                    let normalized = normalize_preformatted(&text);
                    let mut segs: Vec<&str> = normalized.split('\n').collect();
                    let is_last_piece = piece_idx + 1 == total_pieces;
                    if is_last_piece && segs.len() > 1 && segs.last() == Some(&"") {
                        segs.pop();
                    }
                    let seg_count = segs.len();
                    for (seg_idx, seg) in segs.into_iter().enumerate() {
                        tokens.push(InlineToken::PreSegment {
                            text: seg.to_string(),
                            newline_before: seg_idx > 0,
                            node_id,
                            style,
                            href: href.clone(),
                            span_id,
                            left_pad: if seg_idx == 0 { left_pad } else { 0.0 },
                            left_border: if seg_idx == 0 { left_border } else { 0.0 },
                            right_pad: if seg_idx + 1 == seg_count {
                                right_pad
                            } else {
                                0.0
                            },
                            right_border: if seg_idx + 1 == seg_count {
                                right_border
                            } else {
                                0.0
                            },
                        });
                    }
                } else {
                    let collapsed = collapse_html_whitespace(&text);
                    let mut raw_tokens: Vec<(&str, bool)> = Vec::new();
                    let mut word_start: Option<usize> = None;

                    for (idx, ch) in collapsed.char_indices() {
                        if ch == ' ' {
                            if let Some(ws) = word_start.take() {
                                raw_tokens.push((&collapsed[ws..idx], false));
                            }
                            raw_tokens.push((" ", true));
                        } else if word_start.is_none() {
                            word_start = Some(idx);
                        }
                    }
                    if let Some(ws) = word_start {
                        raw_tokens.push((&collapsed[ws..], false));
                    }

                    let tok_count = raw_tokens.len();
                    for (tok_idx, (tok, is_space)) in raw_tokens.into_iter().enumerate() {
                        let lp = if tok_idx == 0 { left_pad } else { 0.0 };
                        let lb = if tok_idx == 0 { left_border } else { 0.0 };
                        let rp = if tok_idx + 1 == tok_count { right_pad } else { 0.0 };
                        let rb = if tok_idx + 1 == tok_count {
                            right_border
                        } else {
                            0.0
                        };

                        if is_space {
                            tokens.push(InlineToken::Space {
                                node_id,
                                style,
                                href: href.clone(),
                                span_id,
                                left_pad: lp,
                                left_border: lb,
                                right_pad: rp,
                                right_border: rb,
                            });
                        } else {
                            tokens.push(InlineToken::Word {
                                text: tok.to_string(),
                                node_id,
                                style,
                                href: href.clone(),
                                span_id,
                                left_pad: lp,
                                left_border: lb,
                                right_pad: rp,
                                right_border: rb,
                            });
                        }
                    }
                }
            }
        }
    }

    tokens
}

fn collapse_html_whitespace(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_ws = false;
    for ch in text.chars() {
        if matches!(ch, ' ' | '\t' | '\n' | '\r' | '\x0C') {
            if !in_ws {
                out.push(' ');
                in_ws = true;
            }
        } else {
            in_ws = false;
            out.push(ch);
        }
    }
    out
}

fn normalize_preformatted(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push('\n');
            }
            '\t' => out.push_str("    "),
            _ => out.push(ch),
        }
    }
    out
}

struct PendingSpace {
    node_id: NodeId,
    style: ComputedStyle,
    href: Option<String>,
    span_id: Option<usize>,
    left_pad: f32,
    left_border: f32,
    right_pad: f32,
    right_border: f32,
}

fn flush_pending_space(
    pending_space: &mut Option<PendingSpace>,
    current_line: &mut Vec<LineItem>,
    line_x: &mut f32,
) {
    if let Some(sp) = pending_space.take() {
        let sp_lh = font::line_height_px(sp.style.font_size);
        let sp_text_w = measure_text(" ", sp.style);
        let sp_total = sp.left_pad + sp.left_border + sp_text_w + sp.right_pad + sp.right_border;
        push_fragment(
            current_line,
            TextLineFragment {
                text: " ".to_string(),
                node_id: Some(sp.node_id),
                style: sp.style,
                href: sp.href,
                span_id: sp.span_id,
                line_height: sp_lh,
                text_width: sp_text_w,
                left_pad: sp.left_pad,
                left_border: sp.left_border,
                right_pad: sp.right_pad,
                right_border: sp.right_border,
            },
        );
        *line_x += sp_total;
    }
}

fn layout_inline_sequence(
    ctx: &LayoutCtx<'_>,
    inline_nodes: &[&StyledNode],
    start_x: f32,
    start_y: f32,
    max_width: f32,
    containing_height: Option<f32>,
) -> (Vec<LayoutBox>, f32) {
    let base_decor = InlineDecor {
        span_id: None,
        background_color: None,
        border_color: Color::rgb(0, 0, 0),
        border_radius: 0.0,
        padding: EdgeSizes::default(),
        border: EdgeSizes::default(),
    };
    let mut next_span_uid = 0usize;
    let mut pieces = Vec::new();
    for node in inline_nodes {
        collect_inline_pieces(ctx.doc, node, &base_decor, &mut next_span_uid, &mut pieces);
    }

    let tokens = tokenize_pieces(pieces);
    if tokens.is_empty() {
        return (Vec::new(), 0.0);
    }

    let avail_w = max_width.max(1.0);
    let mut boxes: Vec<LayoutBox> = Vec::new();
    let mut current_line: Vec<LineItem> = Vec::new();
    let mut line_x = 0.0_f32;
    let mut cur_y = start_y;
    let mut pending_space: Option<PendingSpace> = None;

    for token in tokens {
        match token {
            InlineToken::LineBreak { line_height } => {
                pending_space = None;
                let row_h = flush_line(&mut current_line, start_x, cur_y, &mut boxes)
                    .max(line_height);
                cur_y += row_h;
                line_x = 0.0;
            }
            InlineToken::Space {
                node_id,
                style,
                href,
                span_id,
                left_pad,
                left_border,
                right_pad,
                right_border,
            } => {
                if !current_line.is_empty() && pending_space.is_none() {
                    pending_space = Some(PendingSpace {
                        node_id,
                        style,
                        href,
                        span_id,
                        left_pad,
                        left_border,
                        right_pad,
                        right_border,
                    });
                }
            }
            InlineToken::AtomicBox { node } => {
                let atomic = layout_inline_block_atomic(ctx, node, avail_w, containing_height);
                let item_w = atomic.dimensions.margin_box().width;
                let space_w = pending_space.as_ref().map_or(0.0, |sp| {
                    sp.left_pad
                        + sp.left_border
                        + measure_text(" ", sp.style)
                        + sp.right_pad
                        + sp.right_border
                });

                if !current_line.is_empty() && line_x + space_w + item_w > avail_w + 0.01 {
                    pending_space = None;
                    let row_h = flush_line(&mut current_line, start_x, cur_y, &mut boxes);
                    cur_y += row_h;
                    line_x = 0.0;
                } else {
                    flush_pending_space(&mut pending_space, &mut current_line, &mut line_x);
                }

                current_line.push(LineItem::Atomic(atomic));
                line_x += item_w;
            }
            InlineToken::Word {
                text,
                node_id,
                style,
                href,
                span_id,
                left_pad,
                left_border,
                right_pad,
                right_border,
            } => {
                let lh = font::line_height_px(style.font_size);
                let word_text_w = measure_text(&text, style);
                let left_extra = left_pad + left_border;
                let right_extra = right_pad + right_border;
                let total_word_w = left_extra + word_text_w + right_extra;

                let space_w = pending_space.as_ref().map_or(0.0, |sp| {
                    sp.left_pad
                        + sp.left_border
                        + measure_text(" ", sp.style)
                        + sp.right_pad
                        + sp.right_border
                });

                if !current_line.is_empty() && line_x + space_w + total_word_w > avail_w + 0.01 {
                    pending_space = None;
                    let row_h = flush_line(&mut current_line, start_x, cur_y, &mut boxes);
                    cur_y += row_h;
                    line_x = 0.0;
                } else {
                    flush_pending_space(&mut pending_space, &mut current_line, &mut line_x);
                }

                if line_x + total_word_w <= avail_w + 0.01 {
                    push_fragment(
                        &mut current_line,
                        TextLineFragment {
                            text,
                            node_id: Some(node_id),
                            style,
                            href,
                            span_id,
                            line_height: lh,
                            text_width: word_text_w,
                            left_pad,
                            left_border,
                            right_pad,
                            right_border,
                        },
                    );
                    line_x += total_word_w;
                } else {
                    place_wrapped_chunks(
                        &text,
                        Some(node_id),
                        style,
                        &href,
                        span_id,
                        left_pad,
                        left_border,
                        right_pad,
                        right_border,
                        lh,
                        avail_w,
                        start_x,
                        &mut current_line,
                        &mut line_x,
                        &mut cur_y,
                        &mut boxes,
                        false,
                    );
                }
            }
            InlineToken::PreSegment {
                text,
                newline_before,
                node_id,
                style,
                href,
                span_id,
                left_pad,
                left_border,
                right_pad,
                right_border,
            } => {
                pending_space = None;
                let lh = font::line_height_px(style.font_size);
                if newline_before {
                    if current_line.is_empty() {
                        current_line.push(LineItem::Text(TextLineFragment {
                            text: String::new(),
                            node_id: Some(node_id),
                            style,
                            href: href.clone(),
                            span_id,
                            line_height: lh,
                            text_width: 0.0,
                            left_pad: 0.0,
                            left_border: 0.0,
                            right_pad: 0.0,
                            right_border: 0.0,
                        }));
                    }
                    let row_h = flush_line(&mut current_line, start_x, cur_y, &mut boxes);
                    cur_y += row_h;
                    line_x = 0.0;
                }

                if text.is_empty() {
                    if current_line.is_empty() {
                        current_line.push(LineItem::Text(TextLineFragment {
                            text: String::new(),
                            node_id: Some(node_id),
                            style,
                            href,
                            span_id,
                            line_height: lh,
                            text_width: 0.0,
                            left_pad,
                            left_border,
                            right_pad,
                            right_border,
                        }));
                    }
                    continue;
                }

                let seg_w = measure_text(&text, style);
                let total_w = left_pad + left_border + seg_w + right_pad + right_border;
                if line_x + total_w <= avail_w + 0.01 {
                    push_fragment(
                        &mut current_line,
                        TextLineFragment {
                            text,
                            node_id: Some(node_id),
                            style,
                            href,
                            span_id,
                            line_height: lh,
                            text_width: seg_w,
                            left_pad,
                            left_border,
                            right_pad,
                            right_border,
                        },
                    );
                    line_x += total_w;
                } else {
                    place_wrapped_chunks(
                        &text,
                        Some(node_id),
                        style,
                        &href,
                        span_id,
                        left_pad,
                        left_border,
                        right_pad,
                        right_border,
                        lh,
                        avail_w,
                        start_x,
                        &mut current_line,
                        &mut line_x,
                        &mut cur_y,
                        &mut boxes,
                        true,
                    );
                }
            }
        }
    }

    if !current_line.is_empty() {
        let row_h = flush_line(&mut current_line, start_x, cur_y, &mut boxes);
        cur_y += row_h;
    }

    let merged = merge_multiline_text_boxes(boxes);
    let total_height = (cur_y - start_y).max(0.0);
    (merged, total_height)
}

#[allow(clippy::too_many_arguments)]
fn place_wrapped_chunks(
    text: &str,
    node_id: Option<NodeId>,
    style: ComputedStyle,
    href: &Option<String>,
    span_id: Option<usize>,
    left_pad: f32,
    left_border: f32,
    right_pad: f32,
    right_border: f32,
    lh: f32,
    avail_w: f32,
    start_x: f32,
    current_line: &mut Vec<LineItem>,
    line_x: &mut f32,
    cur_y: &mut f32,
    boxes: &mut Vec<LayoutBox>,
    prefer_space_break: bool,
) {
    let mut rest = text;
    let mut is_first = true;

    while !rest.is_empty() {
        let lp = if is_first { left_pad } else { 0.0 };
        let lb = if is_first { left_border } else { 0.0 };
        let remaining = (avail_w - *line_x - lp - lb).max(0.0);

        let mut split_byte = find_fit_prefix_end(rest, style, remaining, prefer_space_break);

        if split_byte == 0 {
            if !current_line.is_empty() {
                let row_h = flush_line(current_line, start_x, *cur_y, boxes);
                *cur_y += row_h;
                *line_x = 0.0;
                continue;
            }
            if let Some(first_ch) = rest.chars().next() {
                split_byte = first_ch.len_utf8();
            } else {
                break;
            }
        }

        let chunk = &rest[..split_byte];
        rest = &rest[split_byte..];
        let is_last = rest.is_empty();
        let rp = if is_last { right_pad } else { 0.0 };
        let rb = if is_last { right_border } else { 0.0 };
        let chunk_w = measure_text(chunk, style);

        push_fragment(
            current_line,
            TextLineFragment {
                text: chunk.to_string(),
                node_id,
                style,
                href: href.clone(),
                span_id,
                line_height: lh,
                text_width: chunk_w,
                left_pad: lp,
                left_border: lb,
                right_pad: rp,
                right_border: rb,
            },
        );
        *line_x += lp + lb + chunk_w + rp + rb;
        is_first = false;

        if !rest.is_empty() {
            let row_h = flush_line(current_line, start_x, *cur_y, boxes);
            *cur_y += row_h;
            *line_x = 0.0;
        }
    }
}

fn find_fit_prefix_end(
    s: &str,
    style: ComputedStyle,
    max_w: f32,
    prefer_space_break: bool,
) -> usize {
    if s.len() <= 64 {
        let mut last_fit = 0usize;
        let mut last_space_after = 0usize;
        for (idx, ch) in s.char_indices() {
            let next_end = idx + ch.len_utf8();
            let w = measure_text(&s[..next_end], style);
            if w <= max_w + 0.01 {
                last_fit = next_end;
                if prefer_space_break && ch == ' ' {
                    last_space_after = next_end;
                }
            } else {
                break;
            }
        }
        return if prefer_space_break && last_fit < s.len() && last_space_after > 0 {
            last_space_after
        } else {
            last_fit
        };
    }

    let boundaries: Vec<usize> = s
        .char_indices()
        .map(|(i, c)| i + c.len_utf8())
        .collect();
    let mut lo = 0usize;
    let mut hi = boundaries.len();
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let end = boundaries[mid - 1];
        if measure_text(&s[..end], style) <= max_w + 0.01 {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    let last_fit = if lo == 0 { 0 } else { boundaries[lo - 1] };
    if prefer_space_break && last_fit < s.len() {
        if let Some(sp) = s[..last_fit].rfind(' ') {
            return sp + 1;
        }
    }
    last_fit
}

fn inline_text_styles_match(a: ComputedStyle, b: ComputedStyle) -> bool {
    a.color == b.color
        && a.background_color == b.background_color
        && a.border_color == b.border_color
        && (a.border_radius - b.border_radius).abs() < 0.01
        && (a.font_size - b.font_size).abs() < 0.01
        && a.bold == b.bold
        && a.monospace == b.monospace
        && a.text_underline == b.text_underline
        && a.preformatted == b.preformatted
        && a.padding == b.padding
        && a.border == b.border
}

fn can_merge_on_line(a: &TextLineFragment, b: &TextLineFragment) -> bool {
    a.node_id == b.node_id
        && a.href == b.href
        && a.span_id == b.span_id
        && (a.line_height - b.line_height).abs() < 0.01
        && inline_text_styles_match(a.style, b.style)
        && a.right_pad == 0.0
        && a.right_border == 0.0
        && b.left_pad == 0.0
        && b.left_border == 0.0
}

fn push_fragment(line: &mut Vec<LineItem>, frag: TextLineFragment) {
    if let Some(LineItem::Text(prev)) = line.last_mut() {
        if can_merge_on_line(prev, &frag) {
            prev.text.push_str(&frag.text);
            prev.text_width = measure_text(&prev.text, prev.style);
            prev.right_pad = frag.right_pad;
            prev.right_border = frag.right_border;
            return;
        }
    }
    line.push(LineItem::Text(frag));
}

fn find_last_baseline_offset_from_margin_top(box_node: &LayoutBox) -> Option<f32> {
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

fn flush_line(
    line: &mut Vec<LineItem>,
    start_x: f32,
    line_y: f32,
    out: &mut Vec<LayoutBox>,
) -> f32 {
    if line.is_empty() {
        return 0.0;
    }

    let mut max_above = 0.0_f32;
    let mut max_below = 0.0_f32;
    let mut metrics: Vec<(f32, f32)> = Vec::with_capacity(line.len());

    for item in line.iter() {
        let (above, below) = match item {
            LineItem::Text(frag) => {
                let y_pad = ((frag.line_height - frag.style.font_size) * 0.5).max(0.0);
                let baseline_in_content = y_pad + frag.style.font_size * 0.80;
                let above = frag.style.border.top + frag.style.padding.top + baseline_in_content;
                let below = (frag.line_height - baseline_in_content)
                    + frag.style.padding.bottom
                    + frag.style.border.bottom;
                (above, below)
            }
            LineItem::Atomic(b) => {
                let total_h = b.dimensions.margin_box().height;
                let above = match &b.box_type {
                    BoxType::Image { .. } => total_h,
                    _ => find_last_baseline_offset_from_margin_top(b).unwrap_or(total_h),
                };
                let below = (total_h - above).max(0.0);
                (above, below)
            }
        };
        max_above = max_above.max(above);
        max_below = max_below.max(below);
        metrics.push((above, below));
    }

    let row_height = max_above + max_below;
    let mut x_cursor = start_x;

    for (idx, item) in line.drain(..).enumerate() {
        let (above, _) = metrics[idx];
        match item {
            LineItem::Text(frag) => {
                let content_x = x_cursor + frag.left_border + frag.left_pad;
                let border_top_y = line_y + max_above - above;
                let content_y = border_top_y + frag.style.border.top + frag.style.padding.top;
                let padding = EdgeSizes {
                    top: frag.style.padding.top,
                    bottom: frag.style.padding.bottom,
                    left: frag.left_pad,
                    right: frag.right_pad,
                };
                let border = EdgeSizes {
                    top: frag.style.border.top,
                    bottom: frag.style.border.bottom,
                    left: frag.left_border,
                    right: frag.right_border,
                };
                let mut box_style = frag.style;
                box_style.padding = padding;
                box_style.border = border;

                out.push(LayoutBox {
                    node_id: frag.node_id,
                    dimensions: Dimensions {
                        content: Rect {
                            x: content_x,
                            y: content_y,
                            width: frag.text_width,
                            height: frag.line_height,
                        },
                        padding,
                        border,
                        margin: EdgeSizes::default(),
                    },
                    box_type: BoxType::Text {
                        lines: vec![frag.text],
                        line_height: frag.line_height,
                        font_size: frag.style.font_size,
                        font_scale: font_scale_for(frag.style.font_size),
                        monospace: frag.style.monospace,
                    },
                    style: box_style,
                    href: frag.href,
                    children: Vec::new(),
                });

                x_cursor = content_x + frag.text_width + frag.right_pad + frag.right_border;
            }
            LineItem::Atomic(mut b) => {
                let mb = b.dimensions.margin_box();
                let target_mb_x = x_cursor;
                let target_mb_y = line_y + max_above - above;
                shift_layout_box(&mut b, target_mb_x - mb.x, target_mb_y - mb.y);
                x_cursor += mb.width;
                out.push(b);
            }
        }
    }

    row_height
}

fn merge_multiline_text_boxes(boxes: Vec<LayoutBox>) -> Vec<LayoutBox> {
    let mut out: Vec<LayoutBox> = Vec::with_capacity(boxes.len());

    for mut b in boxes {
        if let Some(prev) = out.last_mut() {
            if can_merge_multiline(prev, &b) {
                let b_w = b.dimensions.content.width;
                let b_h = b.dimensions.content.height;
                if let (
                    BoxType::Text {
                        lines: prev_lines, ..
                    },
                    BoxType::Text { lines: cur_lines, .. },
                ) = (&mut prev.box_type, &mut b.box_type)
                {
                    prev_lines.append(cur_lines);
                    prev.dimensions.content.width = prev.dimensions.content.width.max(b_w);
                    prev.dimensions.content.height += b_h;
                    continue;
                }
            }
        }
        out.push(b);
    }

    out
}

fn can_merge_multiline(prev: &LayoutBox, cur: &LayoutBox) -> bool {
    if prev.node_id != cur.node_id || prev.href.is_some() || cur.href.is_some() {
        return false;
    }
    if prev.style.background_color.is_some() || cur.style.background_color.is_some() {
        return false;
    }
    if prev.dimensions.padding != EdgeSizes::default()
        || prev.dimensions.border != EdgeSizes::default()
        || cur.dimensions.padding != EdgeSizes::default()
        || cur.dimensions.border != EdgeSizes::default()
    {
        return false;
    }
    if !inline_text_styles_match(prev.style, cur.style) {
        return false;
    }
    let (
        BoxType::Text {
            line_height: lh1,
            font_size: fs1,
            monospace: m1,
            ..
        },
        BoxType::Text {
            line_height: lh2,
            font_size: fs2,
            monospace: m2,
            ..
        },
    ) = (&prev.box_type, &cur.box_type)
    else {
        return false;
    };
    m1 == m2
        && (fs1 - fs2).abs() < 0.01
        && (lh1 - lh2).abs() < 0.01
        && (prev.dimensions.content.x - cur.dimensions.content.x).abs() < 0.01
        && ((prev.dimensions.content.y + prev.dimensions.content.height)
            - cur.dimensions.content.y)
            .abs()
            < 0.01
}

// ============================================================================
// Two-Pass Flexbox (`display: flex`)
// ============================================================================

fn is_skipped_flex_child(doc: &Document, child: &StyledNode) -> bool {
    if child.style.display == Display::None {
        return true;
    }
    if let Some(NodeKind::Text(t)) = doc.get(child.node_id).map(|n| &n.kind) {
        if !child.style.preformatted && t.trim().is_empty() {
            return true;
        }
    }
    false
}

struct FlexItemMeasure<'a> {
    styled: &'a StyledNode,
    is_text_node: bool,
    base_content_main: f32,
    final_content_main: f32,
    cross_pad_border: f32,
    cross_margin: f32,
    flex_grow: f32,
    flex_shrink: f32,
    hypothetical_outer_main: f32,
    auto_main_start: bool,
    auto_main_end: bool,
    layout: Option<LayoutBox>,
    outer_main: f32,
    outer_cross: f32,
    baseline_offset: f32,
}

fn layout_flex_item(
    ctx: &LayoutCtx<'_>,
    styled: &StyledNode,
    is_text_node: bool,
    avail_width: f32,
    containing_height: Option<f32>,
    forced_w: Option<f32>,
    forced_h: Option<f32>,
) -> LayoutBox {
    if is_text_node {
        let content_w = forced_w.unwrap_or_else(|| {
            let (_, max_w) = measure_intrinsic_content_width(ctx, styled, avail_width);
            max_w.min(avail_width.max(1.0))
        });
        let (children, h) =
            layout_inline_sequence(ctx, &[styled], 0.0, 0.0, content_w, containing_height);
        let final_h = forced_h.map_or(h, |fh| fh.max(h));
        return LayoutBox {
            node_id: Some(styled.node_id),
            dimensions: Dimensions {
                content: Rect {
                    x: 0.0,
                    y: 0.0,
                    width: content_w,
                    height: final_h,
                },
                ..Dimensions::default()
            },
            box_type: BoxType::AnonymousBlock,
            style: styled.style,
            href: styled.href.clone(),
            children,
        };
    }

    let container = Rect {
        x: 0.0,
        y: 0.0,
        width: avail_width.max(0.0),
        height: containing_height.unwrap_or(0.0),
    };
    let mut laid = layout_box_node(
        ctx,
        styled,
        container,
        0.0,
        containing_height,
        forced_w,
        forced_h,
    );
    if laid.style.margin_left_auto {
        laid.dimensions.margin.left = 0.0;
    }
    if laid.style.margin_right_auto {
        laid.dimensions.margin.right = 0.0;
    }
    laid
}

fn layout_flex(
    ctx: &LayoutCtx<'_>,
    styled: &StyledNode,
    containing_rect: Rect,
    border_box_y: f32,
    containing_height: Option<f32>,
    forced_content_w: Option<f32>,
    forced_content_h: Option<f32>,
) -> LayoutBox {
    let (content_x, content_w, margin) =
        compute_block_horizontal(styled.style, containing_rect, forced_content_w);
    let padding = styled.style.padding;
    let border = styled.style.border;
    let content_y = border_box_y + border.top + padding.top;

    let explicit_container_h =
        forced_content_h.or_else(|| resolve_height(styled.style, containing_height));

    let is_row = matches!(
        styled.style.flex_direction,
        FlexDirection::Row | FlexDirection::RowReverse
    );
    let is_reverse = matches!(
        styled.style.flex_direction,
        FlexDirection::RowReverse | FlexDirection::ColumnReverse
    );
    let gap = styled.style.gap.max(0.0);

    let container_main_avail = if is_row {
        Some(content_w)
    } else {
        explicit_container_h
    };
    let container_cross_definite = if is_row {
        explicit_container_h
    } else {
        Some(content_w)
    };

    let mut items: Vec<FlexItemMeasure<'_>> = Vec::new();
    for child in &styled.children {
        if is_skipped_flex_child(ctx.doc, child) {
            continue;
        }
        let is_text_node = matches!(
            ctx.doc.get(child.node_id).map(|n| &n.kind),
            Some(NodeKind::Text(_))
        );
        let s = child.style;
        let ml = if s.margin_left_auto { 0.0 } else { s.margin.left };
        let mr = if s.margin_right_auto {
            0.0
        } else {
            s.margin.right
        };

        let (main_pad_border, main_margin, cross_pad_border, cross_margin) = if is_row {
            (
                s.padding.left + s.padding.right + s.border.left + s.border.right,
                ml + mr,
                s.padding.top + s.padding.bottom + s.border.top + s.border.bottom,
                s.margin.top + s.margin.bottom,
            )
        } else {
            (
                s.padding.top + s.padding.bottom + s.border.top + s.border.bottom,
                s.margin.top + s.margin.bottom,
                s.padding.left + s.padding.right + s.border.left + s.border.right,
                ml + mr,
            )
        };

        // Pass 1: measure base main size
        let basis_main = match s.flex_basis {
            FlexBasis::Px(px) => Some(px.max(0.0)),
            FlexBasis::Percent(pct) => {
                container_main_avail.map(|avail| (avail * (pct / 100.0)).max(0.0))
            }
            FlexBasis::Auto => None,
        };

        let base_content_main = if let Some(b) = basis_main {
            b
        } else if is_row {
            if let Some(w) = resolve_width(s, content_w) {
                let mw = resolve_max_width(s, content_w);
                if let Some(m) = mw { w.min(m) } else { w }.max(0.0)
            } else if is_img_element(ctx.doc, child.node_id) {
                let (_, _, w, _, _) =
                    resolve_image_content_size(ctx, child, content_w, explicit_container_h, None, None);
                w
            } else {
                let avail_c = (content_w - main_pad_border - main_margin).max(0.0);
                let (_, max_w) = measure_intrinsic_content_width(ctx, child, avail_c);
                let mut w = max_w.min(avail_c);
                if let Some(mw) = resolve_max_width(s, content_w) {
                    w = w.min(mw);
                }
                w.max(0.0)
            }
        } else if let Some(h) = resolve_height(s, explicit_container_h) {
            h.max(0.0)
        } else {
            let cross_w = compute_column_item_cross_width(
                ctx,
                child,
                content_w,
                cross_pad_border,
                cross_margin,
                styled.style.align_items,
            );
            let tentative = layout_flex_item(
                ctx,
                child,
                is_text_node,
                content_w,
                explicit_container_h,
                Some(cross_w),
                None,
            );
            tentative.dimensions.content.height.max(0.0)
        };

        let hypothetical_outer_main = base_content_main + main_pad_border + main_margin;

        items.push(FlexItemMeasure {
            styled: child,
            is_text_node,
            base_content_main,
            final_content_main: base_content_main,
            cross_pad_border,
            cross_margin,
            flex_grow: s.flex_grow.max(0.0),
            flex_shrink: s.flex_shrink.max(0.0),
            hypothetical_outer_main,
            auto_main_start: is_row && s.margin_left_auto,
            auto_main_end: is_row && s.margin_right_auto,
            layout: None,
            outer_main: hypothetical_outer_main,
            outer_cross: 0.0,
            baseline_offset: 0.0,
        });
    }

    if items.is_empty() {
        let content_h = explicit_container_h.unwrap_or(0.0);
        return LayoutBox {
            node_id: Some(styled.node_id),
            dimensions: Dimensions {
                content: Rect {
                    x: content_x,
                    y: content_y,
                    width: content_w,
                    height: content_h,
                },
                padding,
                border,
                margin,
            },
            box_type: BoxType::Flex,
            style: styled.style,
            href: styled.href.clone(),
            children: Vec::new(),
        };
    }

    // Break items into flex lines
    let mut lines: Vec<Vec<usize>> = Vec::new();
    let wraps = styled.style.flex_wrap != FlexWrap::NoWrap;
    if !wraps || container_main_avail.is_none() {
        lines.push((0..items.len()).collect());
    } else {
        let avail_main = container_main_avail.unwrap_or(content_w);
        let mut cur_line: Vec<usize> = Vec::new();
        let mut cur_used = 0.0_f32;
        for (idx, it) in items.iter().enumerate() {
            let added = if cur_line.is_empty() {
                it.hypothetical_outer_main
            } else {
                gap + it.hypothetical_outer_main
            };
            if !cur_line.is_empty() && cur_used + added > avail_main + 0.01 {
                lines.push(cur_line);
                cur_line = vec![idx];
                cur_used = it.hypothetical_outer_main;
            } else {
                cur_line.push(idx);
                cur_used += added;
            }
        }
        if !cur_line.is_empty() {
            lines.push(cur_line);
        }
    }

    // Resolve flex-grow / flex-shrink per line, then lay out each item (Pass 2)
    struct LineSummary {
        indices: Vec<usize>,
        cross_size: f32,
        max_baseline: f32,
    }
    let mut summaries: Vec<LineSummary> = Vec::with_capacity(lines.len());
    let single_line = lines.len() == 1;

    for line_indices in lines {
        let n = line_indices.len();
        let total_gaps = if n > 1 { gap * (n - 1) as f32 } else { 0.0 };
        let sum_base_outer: f32 = line_indices
            .iter()
            .map(|&i| items[i].hypothetical_outer_main)
            .sum();
        let avail_main = container_main_avail.unwrap_or(sum_base_outer + total_gaps);
        let free_space = avail_main - (sum_base_outer + total_gaps);

        if free_space > 0.001 {
            let total_grow: f32 = line_indices.iter().map(|&i| items[i].flex_grow).sum();
            if total_grow > 0.0 {
                let denom = total_grow.max(1.0);
                for &i in &line_indices {
                    let share = free_space * (items[i].flex_grow / denom);
                    items[i].final_content_main = (items[i].base_content_main + share).max(0.0);
                }
            }
        } else if free_space < -0.001 {
            let mut frozen = vec![false; n];
            let mut remaining_overflow = -free_space;
            for _ in 0..n {
                let total_weight: f32 = line_indices
                    .iter()
                    .enumerate()
                    .filter(|&(pos, &i)| {
                        !frozen[pos]
                            && items[i].flex_shrink > 0.0
                            && items[i].final_content_main > 0.0
                    })
                    .map(|(_, &i)| items[i].flex_shrink * items[i].base_content_main.max(1.0))
                    .sum();
                if total_weight <= 0.0 || remaining_overflow <= 0.001 {
                    break;
                }
                let mut newly_frozen = false;
                let mut absorbed = 0.0_f32;
                for (pos, &i) in line_indices.iter().enumerate() {
                    if frozen[pos] || items[i].flex_shrink <= 0.0 || items[i].final_content_main <= 0.0 {
                        continue;
                    }
                    let weight = items[i].flex_shrink * items[i].base_content_main.max(1.0);
                    let delta = remaining_overflow * (weight / total_weight);
                    if delta >= items[i].final_content_main {
                        absorbed += items[i].final_content_main;
                        items[i].final_content_main = 0.0;
                        frozen[pos] = true;
                        newly_frozen = true;
                    } else {
                        absorbed += delta;
                        items[i].final_content_main -= delta;
                    }
                }
                remaining_overflow = (remaining_overflow - absorbed).max(0.0);
                if !newly_frozen {
                    break;
                }
            }
        }

        // Lay out items in this line at final main size
        let mut line_cross = 0.0_f32;
        let mut max_baseline = 0.0_f32;
        for &i in &line_indices {
            let final_main = items[i].final_content_main;
            let laid = if is_row {
                layout_flex_item(
                    ctx,
                    items[i].styled,
                    items[i].is_text_node,
                    content_w,
                    explicit_container_h,
                    Some(final_main),
                    None,
                )
            } else {
                let cross_w = compute_column_item_cross_width(
                    ctx,
                    items[i].styled,
                    content_w,
                    items[i].cross_pad_border,
                    items[i].cross_margin,
                    styled.style.align_items,
                );
                layout_flex_item(
                    ctx,
                    items[i].styled,
                    items[i].is_text_node,
                    content_w,
                    explicit_container_h,
                    Some(cross_w),
                    Some(final_main),
                )
            };

            let mb = laid.dimensions.margin_box();
            let (outer_main, outer_cross) = if is_row {
                (mb.width, mb.height)
            } else {
                (mb.height, mb.width)
            };
            let baseline = if is_row {
                find_last_baseline_offset_from_margin_top(&laid).unwrap_or(outer_cross)
            } else {
                0.0
            };

            line_cross = line_cross.max(outer_cross);
            max_baseline = max_baseline.max(baseline);
            items[i].outer_main = outer_main;
            items[i].outer_cross = outer_cross;
            items[i].baseline_offset = baseline;
            items[i].layout = Some(laid);
        }

        if single_line {
            if let Some(def_cross) = container_cross_definite {
                line_cross = line_cross.max(def_cross);
            }
        }

        // Apply align-items: Stretch for row items with auto height
        if is_row && styled.style.align_items == AlignItems::Stretch {
            for &i in &line_indices {
                let has_explicit_h =
                    resolve_height(items[i].styled.style, explicit_container_h).is_some();
                let is_img = is_img_element(ctx.doc, items[i].styled.node_id);
                if !has_explicit_h && !is_img && items[i].outer_cross < line_cross {
                    let target_content_h =
                        (line_cross - items[i].cross_margin - items[i].cross_pad_border).max(0.0);
                    if let Some(ref mut laid) = items[i].layout {
                        if target_content_h > laid.dimensions.content.height {
                            laid.dimensions.content.height = target_content_h;
                            items[i].outer_cross = laid.dimensions.margin_box().height;
                        }
                    }
                }
            }
        }

        summaries.push(LineSummary {
            indices: line_indices,
            cross_size: line_cross,
            max_baseline,
        });
    }

    if styled.style.flex_wrap == FlexWrap::WrapReverse {
        summaries.reverse();
    }

    // Position all lines and items cleanly
    let mut final_children: Vec<LayoutBox> = Vec::with_capacity(items.len());
    let mut cross_cursor = 0.0_f32;
    let total_lines = summaries.len();

    for (line_idx, summary) in summaries.into_iter().enumerate() {
        let n = summary.indices.len();
        let total_gaps = if n > 1 { gap * (n - 1) as f32 } else { 0.0 };
        let sum_outer_main: f32 = summary.indices.iter().map(|&i| items[i].outer_main).sum();
        let avail_main = container_main_avail.unwrap_or(sum_outer_main + total_gaps);
        let raw_leftover = avail_main - (sum_outer_main + total_gaps);

        let auto_margin_count: usize = summary
            .indices
            .iter()
            .map(|&i| {
                usize::from(items[i].auto_main_start) + usize::from(items[i].auto_main_end)
            })
            .sum();

        let (start_offset, extra_between, auto_margin_unit) =
            if is_row && auto_margin_count > 0 && raw_leftover > 0.0 {
                (0.0, 0.0, raw_leftover / auto_margin_count as f32)
            } else {
                let pos_leftover = raw_leftover.max(0.0);
                let (so, eb) = match styled.style.justify_content {
                    JustifyContent::FlexStart => (0.0, 0.0),
                    JustifyContent::FlexEnd => (raw_leftover, 0.0),
                    JustifyContent::Center => (raw_leftover * 0.5, 0.0),
                    JustifyContent::SpaceBetween => {
                        if n > 1 && pos_leftover > 0.0 {
                            (0.0, pos_leftover / (n - 1) as f32)
                        } else {
                            (0.0, 0.0)
                        }
                    }
                    JustifyContent::SpaceAround => {
                        if n > 0 && pos_leftover > 0.0 {
                            let unit = pos_leftover / n as f32;
                            (unit * 0.5, unit)
                        } else {
                            (pos_leftover * 0.5, 0.0)
                        }
                    }
                    JustifyContent::SpaceEvenly => {
                        if n > 0 && pos_leftover > 0.0 {
                            let unit = pos_leftover / (n + 1) as f32;
                            (unit, unit)
                        } else {
                            (pos_leftover * 0.5, 0.0)
                        }
                    }
                };
                (so, eb, 0.0)
            };

        let mut main_cursor = start_offset;
        for (pos, &i) in summary.indices.iter().enumerate() {
            let lead_auto = if items[i].auto_main_start {
                auto_margin_unit
            } else {
                0.0
            };
            let trail_auto = if items[i].auto_main_end {
                auto_margin_unit
            } else {
                0.0
            };
            main_cursor += lead_auto;

            let item_outer_main = items[i].outer_main;
            let visual_main = if is_reverse {
                avail_main - (main_cursor + item_outer_main)
            } else {
                main_cursor
            };

            let cross_offset = match styled.style.align_items {
                AlignItems::Stretch | AlignItems::FlexStart => 0.0,
                AlignItems::FlexEnd => (summary.cross_size - items[i].outer_cross).max(0.0),
                AlignItems::Center => (summary.cross_size - items[i].outer_cross).max(0.0) * 0.5,
                AlignItems::Baseline => {
                    if is_row {
                        (summary.max_baseline - items[i].baseline_offset).max(0.0)
                    } else {
                        0.0
                    }
                }
            };

            let (target_mb_x, target_mb_y) = if is_row {
                (content_x + visual_main, content_y + cross_cursor + cross_offset)
            } else {
                (content_x + cross_cursor + cross_offset, content_y + visual_main)
            };

            if let Some(mut laid) = items[i].layout.take() {
                let mb = laid.dimensions.margin_box();
                shift_layout_box(&mut laid, target_mb_x - mb.x, target_mb_y - mb.y);
                final_children.push(laid);
            }

            main_cursor += item_outer_main + trail_auto;
            if pos + 1 < n {
                main_cursor += gap + extra_between;
            }
        }

        cross_cursor += summary.cross_size;
        if line_idx + 1 < total_lines {
            cross_cursor += gap;
        }
    }

    let inner_h = if is_row {
        cross_cursor
    } else {
        let mut max_bottom = content_y;
        for child in &final_children {
            let mb = child.dimensions.margin_box();
            max_bottom = max_bottom.max(mb.y + mb.height);
        }
        (max_bottom - content_y).max(0.0)
    };

    let content_h = match explicit_container_h {
        Some(h) => h.max(inner_h),
        None => inner_h,
    };

    LayoutBox {
        node_id: Some(styled.node_id),
        dimensions: Dimensions {
            content: Rect {
                x: content_x,
                y: content_y,
                width: content_w,
                height: content_h,
            },
            padding,
            border,
            margin,
        },
        box_type: BoxType::Flex,
        style: styled.style,
        href: styled.href.clone(),
        children: final_children,
    }
}

fn compute_column_item_cross_width(
    ctx: &LayoutCtx<'_>,
    child: &StyledNode,
    container_w: f32,
    cross_pad_border: f32,
    cross_margin: f32,
    align_items: AlignItems,
) -> f32 {
    let s = child.style;
    if let Some(w) = resolve_width(s, container_w) {
        let mw = resolve_max_width(s, container_w);
        return if let Some(m) = mw { w.min(m) } else { w }.max(0.0);
    }
    let max_avail = (container_w - cross_pad_border - cross_margin).max(0.0);
    if is_img_element(ctx.doc, child.node_id) {
        let (_, _, w, _, _) = resolve_image_content_size(ctx, child, container_w, None, None, None);
        return w;
    }
    let mut w = if align_items == AlignItems::Stretch
        && !s.margin_left_auto
        && !s.margin_right_auto
    {
        max_avail
    } else {
        let (_, max_intrinsic) = measure_intrinsic_content_width(ctx, child, max_avail);
        max_intrinsic.min(max_avail)
    };
    if let Some(mw) = resolve_max_width(s, container_w) {
        w = w.min(mw);
    }
    w.max(0.0)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::css::{self, Stylesheet};
    use crate::style::style_tree;

    fn attrs(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn collect_text_boxes(node: &LayoutBox) -> Vec<&LayoutBox> {
        let mut out = Vec::new();
        if matches!(node.box_type, BoxType::Text { .. }) {
            out.push(node);
        }
        for c in &node.children {
            out.extend(collect_text_boxes(c));
        }
        out
    }

    fn collect_boxes_by_type<'a>(node: &'a LayoutBox, pred: &dyn Fn(&BoxType) -> bool) -> Vec<&'a LayoutBox> {
        let mut out = Vec::new();
        if pred(&node.box_type) {
            out.push(node);
        }
        for c in &node.children {
            out.extend(collect_boxes_by_type(c, pred));
        }
        out
    }

    #[test]
    fn test_margin_auto_and_max_width_centering() {
        let mut doc = Document::new();
        let container = doc.create_element(
            "div",
            attrs(&[(
                "style",
                "display: block; max-width: 500px; padding: 10px; margin: 0 auto;",
            )]),
        );
        let fixed = doc.create_element(
            "div",
            attrs(&[(
                "style",
                "display: block; width: 200px; margin: 10px auto; height: 20px;",
            )]),
        );
        doc.append_child(doc.root(), container);
        doc.append_child(container, fixed);

        let styled = style_tree(&doc, &Stylesheet::default()).unwrap();
        let laid = layout_tree(&doc, &styled, 800.0, 600.0, &HashMap::new());

        let c_box = &laid.children[0];
        assert_eq!(c_box.dimensions.content.width, 500.0);
        assert_eq!(c_box.dimensions.margin.left, 140.0);
        assert_eq!(c_box.dimensions.margin.right, 140.0);
        assert_eq!(c_box.dimensions.content.x, 150.0);

        let f_box = &c_box.children[0];
        assert_eq!(f_box.dimensions.margin.left, 150.0);
        assert_eq!(f_box.dimensions.content.x, 300.0);
    }

    #[test]
    fn test_inline_flow_wrapping_and_hit_test_href() {
        let mut doc = Document::new();
        let p = doc.create_element("p", attrs(&[("style", "margin: 0;")]));
        let t1 = doc.create_text("Read the ");
        let a = doc.create_element("a", attrs(&[("href", "about:roadmap")]));
        let a_txt = doc.create_text("Roadmap");
        let t2 = doc.create_text(" or ");
        let code = doc.create_element("code", HashMap::new());
        let c_txt = doc.create_text("Architecture");
        let t3 = doc.create_text(".");
        doc.append_child(doc.root(), p);
        doc.append_child(p, t1);
        doc.append_child(p, a);
        doc.append_child(a, a_txt);
        doc.append_child(p, t2);
        doc.append_child(p, code);
        doc.append_child(code, c_txt);
        doc.append_child(p, t3);

        let styled = style_tree(&doc, &Stylesheet::default()).unwrap();
        let laid = layout_tree(&doc, &styled, 180.0, 600.0, &HashMap::new());
        let texts = collect_text_boxes(&laid);

        let roadmap_box = texts
            .iter()
            .find(|b| matches!(&b.box_type, BoxType::Text { lines, .. } if lines == &vec!["Roadmap"]))
            .expect("Roadmap box");
        let arch_box = texts
            .iter()
            .find(|b| matches!(&b.box_type, BoxType::Text { lines, .. } if lines == &vec!["Architecture"]))
            .expect("Architecture box");

        let expected_x = font::measure_text_px("Read the ", 16.0, false, false);
        assert!((roadmap_box.dimensions.content.x - expected_x).abs() < 0.05);
        assert!(arch_box.dimensions.content.y > roadmap_box.dimensions.content.y);
        assert_eq!(arch_box.dimensions.padding.left, 4.0);
        assert!(arch_box.style.background_color.is_some());

        let rx = roadmap_box.dimensions.content.x + 10.0;
        let ry = roadmap_box.dimensions.content.y + 5.0;
        assert_eq!(hit_test_href(&laid, rx, ry), Some("about:roadmap"));
        assert_eq!(hit_test_href(&laid, 5.0, ry), None);
    }

    #[test]
    fn test_preformatted_preserves_lines_and_spaces() {
        let mut doc = Document::new();
        let pre = doc.create_element("pre", HashMap::new());
        let txt = doc.create_text("  fn main() {\n\n    println!(\"hi\");\n}");
        doc.append_child(doc.root(), pre);
        doc.append_child(pre, txt);

        let styled = style_tree(&doc, &Stylesheet::default()).unwrap();
        let laid = layout_tree(&doc, &styled, 600.0, 600.0, &HashMap::new());
        let texts = collect_text_boxes(&laid);
        assert_eq!(texts.len(), 1);
        if let BoxType::Text { lines, .. } = &texts[0].box_type {
            assert_eq!(
                lines,
                &vec!["  fn main() {", "", "    println!(\"hi\");", "}"]
            );
        } else {
            panic!("expected Text box");
        }
    }

    #[test]
    fn test_utf8_cyrillic_word_wrapping() {
        let mut doc = Document::new();
        let p = doc.create_element("p", HashMap::new());
        let txt = doc.create_text("Привет мир это тест переноса Длинноесловобезпробелов");
        doc.append_child(doc.root(), p);
        doc.append_child(p, txt);

        let styled = style_tree(&doc, &Stylesheet::default()).unwrap();
        let laid = layout_tree(&doc, &styled, 80.0, 600.0, &HashMap::new());
        let texts = collect_text_boxes(&laid);
        assert_eq!(texts.len(), 1);
        if let BoxType::Text { lines, .. } = &texts[0].box_type {
            assert!(lines.len() >= 4);
            for line in lines {
                assert!(font::measure_text_px(line, 16.0, false, false) <= 80.05);
            }
            let rejoined: String = lines.join("");
            assert!(rejoined.contains("Привет"));
            assert!(rejoined.contains("Длинноесловобезпробелов"));
        } else {
            panic!("expected Text box");
        }
    }

    #[test]
    fn test_dump_layout_contains_boxes_and_href() {
        let mut doc = Document::new();
        let a = doc.create_element("a", attrs(&[("href", "https://example.org")]));
        let t = doc.create_text("Example");
        doc.append_child(doc.root(), a);
        doc.append_child(a, t);

        let styled = style_tree(&doc, &css::parse("")).unwrap();
        let laid = layout_tree(&doc, &styled, 400.0, 600.0, &HashMap::new());
        let dump = dump_layout(&laid);
        assert!(dump.contains("Block"));
        assert!(dump.contains("Text"));
        assert!(dump.contains("href=\"https://example.org\""));
        assert!(dump.contains("\"Example\""));
    }

    #[test]
    fn test_inline_block_side_by_side_and_wrapping() {
        let mut doc = Document::new();
        let box1 = doc.create_element(
            "div",
            attrs(&[("style", "display: inline-block; width: 100px; height: 30px;")]),
        );
        let box2 = doc.create_element(
            "div",
            attrs(&[("style", "display: inline-block; width: 100px; height: 30px;")]),
        );
        doc.append_child(doc.root(), box1);
        doc.append_child(doc.root(), box2);

        let styled = style_tree(&doc, &Stylesheet::default()).unwrap();

        let wide = layout_tree(&doc, &styled, 250.0, 400.0, &HashMap::new());
        let wide_ibs = collect_boxes_by_type(&wide, &|bt| matches!(bt, BoxType::InlineBlock));
        assert_eq!(wide_ibs.len(), 2);
        assert_eq!(wide_ibs[0].dimensions.content.y, wide_ibs[1].dimensions.content.y);
        assert_eq!(wide_ibs[0].dimensions.content.x, 0.0);
        assert_eq!(wide_ibs[1].dimensions.content.x, 100.0);

        let narrow = layout_tree(&doc, &styled, 150.0, 400.0, &HashMap::new());
        let narrow_ibs = collect_boxes_by_type(&narrow, &|bt| matches!(bt, BoxType::InlineBlock));
        assert_eq!(narrow_ibs.len(), 2);
        assert!(narrow_ibs[1].dimensions.content.y >= narrow_ibs[0].dimensions.content.y + 30.0);
        assert_eq!(narrow_ibs[1].dimensions.content.x, 0.0);
    }

    #[test]
    fn test_flexbox_row_space_between_align_center_and_gap() {
        let mut doc = Document::new();
        let flex = doc.create_element(
            "div",
            attrs(&[(
                "style",
                "display: flex; justify-content: space-between; align-items: center; gap: 12px;",
            )]),
        );
        let c1 = doc.create_element("div", attrs(&[("style", "width: 100px; height: 40px;")]));
        let c2 = doc.create_element("div", attrs(&[("style", "width: 100px; height: 80px;")]));
        doc.append_child(doc.root(), flex);
        doc.append_child(flex, c1);
        doc.append_child(flex, c2);

        let styled = style_tree(&doc, &Stylesheet::default()).unwrap();
        let laid = layout_tree(&doc, &styled, 400.0, 400.0, &HashMap::new());
        let flex_box = &laid.children[0];
        assert_eq!(flex_box.box_type, BoxType::Flex);
        assert_eq!(flex_box.children.len(), 2);

        let b1 = &flex_box.children[0];
        let b2 = &flex_box.children[1];
        assert_eq!(b1.dimensions.content.x, 0.0);
        assert_eq!(b1.dimensions.content.y, 20.0);
        assert_eq!(b2.dimensions.content.x, 300.0);
        assert_eq!(b2.dimensions.content.y, 0.0);
    }

    #[test]
    fn test_flexbox_grow_shrink_clamp_wrap_and_column() {
        // flex-grow + gap + flex-shrink clamp >= 0
        let mut doc = Document::new();
        let row_grow = doc.create_element(
            "div",
            attrs(&[("style", "display: flex; width: 412px; gap: 12px;")]),
        );
        let g1 = doc.create_element("div", attrs(&[("style", "width: 100px; height: 20px;")]));
        let g2 = doc.create_element(
            "div",
            attrs(&[("style", "flex-grow: 1; flex-basis: 0px; height: 20px;")]),
        );
        doc.append_child(doc.root(), row_grow);
        doc.append_child(row_grow, g1);
        doc.append_child(row_grow, g2);

        let row_shrink = doc.create_element(
            "div",
            attrs(&[("style", "display: flex; width: 100px;")]),
        );
        let s1 = doc.create_element(
            "div",
            attrs(&[("style", "width: 200px; flex-shrink: 1; height: 20px;")]),
        );
        let s2 = doc.create_element(
            "div",
            attrs(&[("style", "width: 200px; flex-shrink: 1; height: 20px;")]),
        );
        doc.append_child(doc.root(), row_shrink);
        doc.append_child(row_shrink, s1);
        doc.append_child(row_shrink, s2);

        let row_clamp = doc.create_element(
            "div",
            attrs(&[("style", "display: flex; width: 40px;")]),
        );
        let sc1 = doc.create_element(
            "div",
            attrs(&[("style", "width: 50px; padding: 30px; flex-shrink: 1; height: 20px;")]),
        );
        doc.append_child(doc.root(), row_clamp);
        doc.append_child(row_clamp, sc1);

        let wrap_box = doc.create_element(
            "div",
            attrs(&[("style", "display: flex; flex-wrap: wrap; width: 250px; gap: 10px;")]),
        );
        for _ in 0..3 {
            let item =
                doc.create_element("div", attrs(&[("style", "width: 100px; height: 30px;")]));
            doc.append_child(wrap_box, item);
        }
        doc.append_child(doc.root(), wrap_box);

        let col_box = doc.create_element(
            "div",
            attrs(&[("style", "display: flex; flex-direction: column; width: 300px; gap: 10px;")]),
        );
        let col1 = doc.create_element("div", attrs(&[("style", "height: 40px;")]));
        let col2 = doc.create_element("div", attrs(&[("style", "height: 60px;")]));
        doc.append_child(doc.root(), col_box);
        doc.append_child(col_box, col1);
        doc.append_child(col_box, col2);

        let styled = style_tree(&doc, &Stylesheet::default()).unwrap();
        let laid = layout_tree(&doc, &styled, 800.0, 600.0, &HashMap::new());

        let fg = &laid.children[0];
        assert_eq!(fg.children[0].dimensions.content.width, 100.0);
        assert_eq!(fg.children[1].dimensions.content.width, 300.0);
        assert_eq!(fg.children[1].dimensions.content.x, 112.0);

        let fs = &laid.children[1];
        assert!(fs.children[0].dimensions.content.width >= 0.0);
        assert!(fs.children[1].dimensions.content.width >= 0.0);
        assert_eq!(fs.children[0].dimensions.content.width, 50.0);
        assert_eq!(fs.children[1].dimensions.content.width, 50.0);

        let fc_clamp = &laid.children[2];
        assert_eq!(fc_clamp.children[0].dimensions.content.width, 0.0);

        let fw = &laid.children[3];
        assert_eq!(fw.children[0].dimensions.content.x, 0.0);
        assert_eq!(fw.children[1].dimensions.content.x, 110.0);
        assert_eq!(fw.children[2].dimensions.content.x, 0.0);
        assert_eq!(
            fw.children[2].dimensions.content.y,
            fw.children[0].dimensions.content.y + 40.0
        );
        assert_eq!(fw.dimensions.content.height, 70.0);

        let fc = &laid.children[4];
        assert_eq!(fc.children[0].dimensions.content.width, 300.0);
        assert_eq!(fc.children[1].dimensions.content.width, 300.0);
        assert_eq!(
            fc.children[1].dimensions.content.y,
            fc.children[0].dimensions.content.y + 50.0
        );
        assert_eq!(fc.dimensions.content.height, 110.0);
    }

    #[test]
    fn test_image_aspect_ratio_and_alt_fallback() {
        let mut doc = Document::new();
        let img1 = doc.create_element(
            "img",
            attrs(&[("src", "logo.png"), ("width", "100"), ("alt", "Logo")]),
        );
        let img2 = doc.create_element(
            "img",
            attrs(&[("src", "missing.png"), ("alt", "Broken photo")]),
        );
        doc.append_child(doc.root(), img1);
        doc.append_child(doc.root(), img2);

        let mut images = HashMap::new();
        images.insert(
            "logo.png".to_string(),
            Arc::new(DecodedImage {
                width: 200,
                height: 100,
                rgba: vec![255; 200 * 100 * 4],
            }),
        );

        let styled = style_tree(&doc, &Stylesheet::default()).unwrap();
        let laid = layout_tree(&doc, &styled, 600.0, 400.0, &images);
        let imgs = collect_boxes_by_type(&laid, &|bt| matches!(bt, BoxType::Image { .. }));
        assert_eq!(imgs.len(), 2);

        assert_eq!(imgs[0].dimensions.content.width, 100.0);
        assert_eq!(imgs[0].dimensions.content.height, 50.0);
        assert!(matches!(
            &imgs[0].box_type,
            BoxType::Image { image: Some(_), .. }
        ));

        assert!(imgs[1].dimensions.content.width > 20.0);
        assert!(imgs[1].dimensions.content.height >= 20.0);
        assert!(matches!(
            &imgs[1].box_type,
            BoxType::Image { image: None, alt } if alt == "Broken photo"
        ));
    }

    #[test]
    fn test_hit_test_node_returns_deepest_with_ancestor_fallback() {
        let mut doc = Document::new();
        let card = doc.create_element(
            "div",
            attrs(&[("style", "display: block; width: 200px; height: 100px;")]),
        );
        let span = doc.create_element("span", HashMap::new());
        let txt = doc.create_text("Click");
        let block = doc.create_element(
            "div",
            attrs(&[("style", "display: block; width: 100px; height: 20px;")]),
        );
        doc.append_child(doc.root(), card);
        doc.append_child(card, span);
        doc.append_child(span, txt);
        doc.append_child(card, block);

        let styled = style_tree(&doc, &Stylesheet::default()).unwrap();
        let laid = layout_tree(&doc, &styled, 400.0, 400.0, &HashMap::new());

        // Clicking on text inside AnonymousBlock hits the text NodeId
        let texts = collect_text_boxes(&laid);
        let t_rect = texts[0].dimensions.content;
        assert_eq!(
            hit_test_node(&laid, t_rect.x + 2.0, t_rect.y + 2.0),
            Some(txt)
        );
        // Clicking empty space inside card (right of text, below block) falls back to card NodeId
        assert_eq!(hit_test_node(&laid, 180.0, 80.0), Some(card));
        // Clicking outside everything hits root Document (0) or None if outside root
        assert_eq!(hit_test_node(&laid, 500.0, 500.0), None);
    }

    #[test]
    fn test_percentage_sizing_ignores_auto_parent_height() {
        let mut doc = Document::new();
        let auto_parent = doc.create_element("div", attrs(&[("style", "width: 400px;")]));
        let pct_child = doc.create_element(
            "div",
            attrs(&[("style", "width: 50%; height: 50%;")]),
        );
        let def_parent = doc.create_element(
            "div",
            attrs(&[("style", "width: 400px; height: 200px;")]),
        );
        let def_child = doc.create_element(
            "div",
            attrs(&[("style", "width: 50%; height: 50%;")]),
        );
        doc.append_child(doc.root(), auto_parent);
        doc.append_child(auto_parent, pct_child);
        doc.append_child(doc.root(), def_parent);
        doc.append_child(def_parent, def_child);

        let styled = style_tree(&doc, &Stylesheet::default()).unwrap();
        let laid = layout_tree(&doc, &styled, 800.0, 600.0, &HashMap::new());

        let c_auto = &laid.children[0].children[0];
        assert_eq!(c_auto.dimensions.content.width, 200.0);
        assert_eq!(c_auto.dimensions.content.height, 0.0);

        let c_def = &laid.children[1].children[0];
        assert_eq!(c_def.dimensions.content.width, 200.0);
        assert_eq!(c_def.dimensions.content.height, 100.0);
    }
}
