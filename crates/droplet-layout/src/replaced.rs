use std::sync::Arc;

use droplet_dom::{Document, NodeId};
use droplet_gfx::font::line_height_px;
use droplet_gfx::image::DecodedImage;
use droplet_gfx::Rect;
use droplet_style::{Display, EdgeSizes, StyledNode};

use crate::box_model::{
    fixed_horizontal_margins, measure_text, resolve_height, resolve_max_width, resolve_width,
    BoxType, Dimensions, LayoutBox, LayoutCtx,
};

pub(crate) struct ResolvedImageSize {
    pub(crate) image: Option<Arc<DecodedImage>>,
    pub(crate) alt: String,
    pub(crate) width: f32,
    pub(crate) height: f32,
    pub(crate) needs_default_chrome: bool,
}

pub(crate) fn is_img_element(doc: &Document, node_id: NodeId) -> bool {
    doc.element(node_id)
        .is_some_and(|e| e.tag_name.eq_ignore_ascii_case("img"))
}

pub(crate) fn img_attr_px(raw: &str) -> Option<f32> {
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

pub(crate) fn resolve_image_content_size(
    ctx: &LayoutCtx<'_>,
    styled: &StyledNode,
    containing_width: f32,
    containing_height: Option<f32>,
    forced_content_w: Option<f32>,
    forced_content_h: Option<f32>,
) -> ResolvedImageSize {
    let elem = ctx.doc.element(styled.node_id);
    let src = elem.and_then(|e| e.attr("src"));
    let alt = elem.and_then(|e| e.attr("alt")).unwrap_or("").to_string();
    let decoded = lookup_image(ctx, src);

    let css_w = resolve_width(styled.style, containing_width);
    let css_h = resolve_height(styled.style, containing_height);
    let max_w = resolve_max_width(styled.style, containing_width);
    let attr_w = elem.and_then(|e| e.attr("width")).and_then(img_attr_px);
    let attr_h = elem.and_then(|e| e.attr("height")).and_then(img_attr_px);

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
            (
                given_w.unwrap_or(0.0).max(0.0),
                given_h.unwrap_or(0.0).max(0.0),
            )
        };

        if let Some(mw) = max_w {
            if w > mw {
                w = mw.max(0.0);
                if given_h.is_none() && iw > 0.0 && ih > 0.0 {
                    h = (w * ih / iw).max(0.0);
                }
            }
        }
        ResolvedImageSize {
            image: decoded,
            alt,
            width: w,
            height: h,
            needs_default_chrome: false,
        }
    } else {
        let label = if alt.is_empty() { "[img]" } else { alt.as_str() };
        let text_w = measure_text(label, styled.style).max(24.0);
        let text_h = line_height_px(styled.style.font_size).max(20.0);
        let mut w = given_w.unwrap_or(text_w).max(0.0);
        if let Some(mw) = max_w {
            w = w.min(mw).max(0.0);
        }
        let h = given_h.unwrap_or(text_h).max(0.0);
        let needs_default_chrome = given_w.is_none() && given_h.is_none();
        ResolvedImageSize {
            image: None,
            alt,
            width: w,
            height: h,
            needs_default_chrome,
        }
    }
}

pub(crate) fn layout_image_box(
    ctx: &LayoutCtx<'_>,
    styled: &StyledNode,
    containing_rect: Rect,
    border_box_y: f32,
    containing_height: Option<f32>,
    forced_content_w: Option<f32>,
    forced_content_h: Option<f32>,
) -> LayoutBox {
    let resolved = resolve_image_content_size(
        ctx,
        styled,
        containing_rect.width,
        containing_height,
        forced_content_w,
        forced_content_h,
    );

    let mut padding = styled.style.padding;
    let mut border = styled.style.border;
    if resolved.image.is_none() {
        if resolved.needs_default_chrome && padding == EdgeSizes::default() {
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
        let (fixed_left, fixed_right) = fixed_horizontal_margins(styled.style);
        let used_fixed = fixed_left + fixed_right;
        let underflow = containing_rect.width - non_content - resolved.width - used_fixed;
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
                width: resolved.width,
                height: resolved.height,
            },
            padding,
            border,
            margin,
        },
        box_type: BoxType::Image {
            image: resolved.image,
            alt: resolved.alt,
        },
        style: styled.style,
        href: styled.href.clone(),
        children: Vec::new(),
    }
}
