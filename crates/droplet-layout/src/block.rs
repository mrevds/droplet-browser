use droplet_dom::{Document, NodeId};
use droplet_gfx::font::line_height_px;
use droplet_gfx::Rect;
use droplet_style::{ComputedStyle, Display, EdgeSizes, StyledNode};

use crate::box_model::{
    box_dispatch, compute_block_horizontal, contains_block, font_scale_for, is_block_level,
    measure_text, resolve_height, BoxType, Dimensions, LayoutBox, LayoutCtx,
};
use crate::inline;

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

#[derive(Clone, Copy)]
struct AnonBlockBounds {
    x: f32,
    width: f32,
    containing_height: Option<f32>,
}

fn flush_inline_run(
    ctx: &LayoutCtx<'_>,
    inline_run: &mut Vec<&StyledNode>,
    parent_style: ComputedStyle,
    bounds: AnonBlockBounds,
    cursor_y: &mut f32,
    prev_margin_bottom: &mut f32,
    layout_children: &mut Vec<LayoutBox>,
) {
    if inline_run.is_empty() {
        return;
    }
    *cursor_y += *prev_margin_bottom;
    *prev_margin_bottom = 0.0;
    let anon = layout_anonymous_block(
        ctx,
        inline_run,
        parent_style,
        bounds.x,
        *cursor_y,
        bounds.width,
        bounds.containing_height,
    );
    let anon_h = anon.dimensions.border_box().height;
    *cursor_y += anon_h;
    layout_children.push(anon);
    inline_run.clear();
}

pub(crate) fn layout_block(
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
            let (inline_boxes, inline_height) = inline::layout_inline_sequence(
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
        let anon_bounds = AnonBlockBounds {
            x: content_x,
            width: content_w,
            containing_height: explicit_h,
        };
        let mut inline_run: Vec<&StyledNode> = Vec::new();
        for child in visible_children {
            if !contains_block(child) {
                inline_run.push(child);
                continue;
            }
            flush_inline_run(
                ctx,
                &mut inline_run,
                styled.style,
                anon_bounds,
                &mut cursor_y,
                &mut prev_margin_bottom,
                &mut layout_children,
            );

            let child_boxes = flatten_blocks_if_inline_wrapper(child);
            for block_node in child_boxes {
                let gap = collapse_margins(prev_margin_bottom, block_node.style.margin.top);
                let child_bb_y = cursor_y + gap;
                let child_layout = box_dispatch(
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
        flush_inline_run(
            ctx,
            &mut inline_run,
            styled.style,
            anon_bounds,
            &mut cursor_y,
            &mut prev_margin_bottom,
            &mut layout_children,
        );
    }

    if let Some(marker) = list_marker_text(ctx.doc, styled.node_id) {
        let fs = styled.style.font_size;
        let lh = line_height_px(fs);
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
        inline::layout_inline_sequence(ctx, inline_nodes, x, y, width, containing_height);
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
