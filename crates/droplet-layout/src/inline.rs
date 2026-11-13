use droplet_dom::{Document, NodeId, NodeKind};
use droplet_gfx::font::line_height_px;
use droplet_gfx::{Color, Rect};
use droplet_style::{
    ComputedStyle, Display, EdgeSizes, FlexBasis, FlexDirection, FlexWrap, StyledNode,
};

use crate::box_model::{
    contains_block, find_last_baseline_offset_from_margin_top, fixed_horizontal_margins,
    font_scale_for, measure_text, resolve_max_width, resolve_width, shift_layout_box, BoxType,
    Dimensions, LayoutBox, LayoutCtx,
};
use crate::{block, flex, replaced};

fn layout_inline_block_atomic(
    ctx: &LayoutCtx<'_>,
    styled: &StyledNode,
    avail_w: f32,
    containing_height: Option<f32>,
) -> LayoutBox {
    if replaced::is_img_element(ctx.doc, styled.node_id) {
        let container = Rect {
            x: 0.0,
            y: 0.0,
            width: avail_w.max(0.0),
            height: containing_height.unwrap_or(0.0),
        };
        return replaced::layout_image_box(
            ctx,
            styled,
            container,
            0.0,
            containing_height,
            None,
            None,
        );
    }

    let pad = styled.style.padding;
    let bdr = styled.style.border;
    let (ml, mr) = fixed_horizontal_margins(styled.style);
    let margin_h = ml + mr;
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
        flex::layout_flex(
            ctx,
            styled,
            container,
            0.0,
            containing_height,
            Some(content_w),
            None,
        )
    } else {
        block::layout_block(
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

pub(crate) fn measure_intrinsic_content_width(
    ctx: &LayoutCtx<'_>,
    node: &StyledNode,
    avail_w: f32,
) -> (f32, f32) {
    if node.style.display == Display::None {
        return (0.0, 0.0);
    }

    if replaced::is_img_element(ctx.doc, node.node_id) {
        let resolved =
            replaced::resolve_image_content_size(ctx, node, avail_w, None, None, None);
        return (resolved.width, resolved.width);
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
                    if flex::is_skipped_flex_child(ctx.doc, child) {
                        continue;
                    }
                    let (c_min, c_max) = measure_intrinsic_content_width(ctx, child, child_avail);
                    let (ml, mr) = fixed_horizontal_margins(child.style);
                    let extra = child.style.padding.left
                        + child.style.padding.right
                        + child.style.border.left
                        + child.style.border.right
                        + ml
                        + mr;
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
                    let (ml, mr) = fixed_horizontal_margins(child.style);
                    ml + mr
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

#[derive(Clone)]
struct InlineSpanMeta {
    node_id: NodeId,
    style: ComputedStyle,
    href: Option<String>,
    span_id: Option<usize>,
    left_pad: f32,
    left_border: f32,
    right_pad: f32,
    right_border: f32,
}

impl InlineSpanMeta {
    fn space_width(&self) -> f32 {
        self.left_pad
            + self.left_border
            + measure_text(" ", self.style)
            + self.right_pad
            + self.right_border
    }

    fn into_fragment(
        self,
        text: String,
        line_height: f32,
        text_width: f32,
    ) -> TextLineFragment {
        TextLineFragment {
            text,
            node_id: Some(self.node_id),
            style: self.style,
            href: self.href,
            span_id: self.span_id,
            line_height,
            text_width,
            left_pad: self.left_pad,
            left_border: self.left_border,
            right_pad: self.right_pad,
            right_border: self.right_border,
        }
    }
}

enum InlineToken<'a> {
    LineBreak {
        line_height: f32,
    },
    AtomicBox {
        node: &'a StyledNode,
    },
    Space(Box<InlineSpanMeta>),
    Word {
        text: String,
        meta: Box<InlineSpanMeta>,
    },
    PreSegment {
        text: String,
        newline_before: bool,
        meta: Box<InlineSpanMeta>,
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

struct InlineFlowState {
    avail_w: f32,
    start_x: f32,
    current_line: Vec<LineItem>,
    line_x: f32,
    cur_y: f32,
    boxes: Vec<LayoutBox>,
}

impl InlineFlowState {
    fn flush_current_line(&mut self) -> f32 {
        let row_h = flush_line(&mut self.current_line, self.start_x, self.cur_y, &mut self.boxes);
        self.cur_y += row_h;
        self.line_x = 0.0;
        row_h
    }
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
    if node.style.display == Display::InlineBlock || replaced::is_img_element(doc, node.node_id) {
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
                    line_height: line_height_px(node.style.font_size),
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
                    let InlinePiece::Text {
                        span_id,
                        left_pad,
                        left_border,
                        ..
                    } = piece
                    else {
                        continue;
                    };
                    if *span_id == target_id {
                        if seen_first {
                            *left_pad = 0.0;
                            *left_border = 0.0;
                        } else {
                            seen_first = true;
                        }
                    }
                }
                let mut seen_last = false;
                for piece in out[start_idx..].iter_mut().rev() {
                    let InlinePiece::Text {
                        span_id,
                        right_pad,
                        right_border,
                        ..
                    } = piece
                    else {
                        continue;
                    };
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
                        let meta = Box::new(InlineSpanMeta {
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
                        tokens.push(InlineToken::PreSegment {
                            text: seg.to_string(),
                            newline_before: seg_idx > 0,
                            meta,
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
                        let meta = Box::new(InlineSpanMeta {
                            node_id,
                            style,
                            href: href.clone(),
                            span_id,
                            left_pad: if tok_idx == 0 { left_pad } else { 0.0 },
                            left_border: if tok_idx == 0 { left_border } else { 0.0 },
                            right_pad: if tok_idx + 1 == tok_count {
                                right_pad
                            } else {
                                0.0
                            },
                            right_border: if tok_idx + 1 == tok_count {
                                right_border
                            } else {
                                0.0
                            },
                        });

                        if is_space {
                            tokens.push(InlineToken::Space(meta));
                        } else {
                            tokens.push(InlineToken::Word {
                                text: tok.to_string(),
                                meta,
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

fn flush_pending_space(
    pending_space: &mut Option<Box<InlineSpanMeta>>,
    flow: &mut InlineFlowState,
) {
    if let Some(sp) = pending_space.take() {
        let sp_lh = line_height_px(sp.style.font_size);
        let sp_text_w = measure_text(" ", sp.style);
        let sp_total = sp.left_pad + sp.left_border + sp_text_w + sp.right_pad + sp.right_border;
        push_fragment(
            &mut flow.current_line,
            sp.into_fragment(" ".to_string(), sp_lh, sp_text_w),
        );
        flow.line_x += sp_total;
    }
}

pub(crate) fn layout_inline_sequence(
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

    let mut flow = InlineFlowState {
        avail_w: max_width.max(1.0),
        start_x,
        current_line: Vec::new(),
        line_x: 0.0,
        cur_y: start_y,
        boxes: Vec::new(),
    };
    let mut pending_space: Option<Box<InlineSpanMeta>> = None;

    for token in tokens {
        match token {
            InlineToken::LineBreak { line_height } => {
                pending_space = None;
                let row_h = flush_line(
                    &mut flow.current_line,
                    flow.start_x,
                    flow.cur_y,
                    &mut flow.boxes,
                )
                .max(line_height);
                flow.cur_y += row_h;
                flow.line_x = 0.0;
            }
            InlineToken::Space(meta) => {
                if !flow.current_line.is_empty() && pending_space.is_none() {
                    pending_space = Some(meta);
                }
            }
            InlineToken::AtomicBox { node } => {
                let atomic = layout_inline_block_atomic(ctx, node, flow.avail_w, containing_height);
                let item_w = atomic.dimensions.margin_box().width;
                let space_w = pending_space.as_ref().map_or(0.0, |sp| sp.space_width());

                if !flow.current_line.is_empty()
                    && flow.line_x + space_w + item_w > flow.avail_w + 0.01
                {
                    pending_space = None;
                    flow.flush_current_line();
                } else {
                    flush_pending_space(&mut pending_space, &mut flow);
                }

                flow.current_line.push(LineItem::Atomic(atomic));
                flow.line_x += item_w;
            }
            InlineToken::Word { text, meta } => {
                let lh = line_height_px(meta.style.font_size);
                let word_text_w = measure_text(&text, meta.style);
                let total_word_w =
                    meta.left_pad + meta.left_border + word_text_w + meta.right_pad + meta.right_border;
                let space_w = pending_space.as_ref().map_or(0.0, |sp| sp.space_width());

                if !flow.current_line.is_empty()
                    && flow.line_x + space_w + total_word_w > flow.avail_w + 0.01
                {
                    pending_space = None;
                    flow.flush_current_line();
                } else {
                    flush_pending_space(&mut pending_space, &mut flow);
                }

                if flow.line_x + total_word_w <= flow.avail_w + 0.01 {
                    push_fragment(&mut flow.current_line, meta.into_fragment(text, lh, word_text_w));
                    flow.line_x += total_word_w;
                } else {
                    place_wrapped_chunks(&text, *meta, lh, &mut flow, false);
                }
            }
            InlineToken::PreSegment {
                text,
                newline_before,
                meta,
            } => {
                pending_space = None;
                let lh = line_height_px(meta.style.font_size);
                if newline_before {
                    if flow.current_line.is_empty() {
                        flow.current_line.push(LineItem::Text(TextLineFragment {
                            text: String::new(),
                            node_id: Some(meta.node_id),
                            style: meta.style,
                            href: meta.href.clone(),
                            span_id: meta.span_id,
                            line_height: lh,
                            text_width: 0.0,
                            left_pad: 0.0,
                            left_border: 0.0,
                            right_pad: 0.0,
                            right_border: 0.0,
                        }));
                    }
                    flow.flush_current_line();
                }

                if text.is_empty() {
                    if flow.current_line.is_empty() {
                        flow.current_line
                            .push(LineItem::Text(meta.into_fragment(String::new(), lh, 0.0)));
                    }
                    continue;
                }

                let seg_w = measure_text(&text, meta.style);
                let total_w =
                    meta.left_pad + meta.left_border + seg_w + meta.right_pad + meta.right_border;
                if flow.line_x + total_w <= flow.avail_w + 0.01 {
                    push_fragment(&mut flow.current_line, meta.into_fragment(text, lh, seg_w));
                    flow.line_x += total_w;
                } else {
                    place_wrapped_chunks(&text, *meta, lh, &mut flow, true);
                }
            }
        }
    }

    if !flow.current_line.is_empty() {
        flow.flush_current_line();
    }

    let merged = merge_multiline_text_boxes(flow.boxes);
    let total_height = (flow.cur_y - start_y).max(0.0);
    (merged, total_height)
}

fn place_wrapped_chunks(
    text: &str,
    meta: InlineSpanMeta,
    lh: f32,
    flow: &mut InlineFlowState,
    prefer_space_break: bool,
) {
    let mut rest = text;
    let mut is_first = true;

    while !rest.is_empty() {
        let lp = if is_first { meta.left_pad } else { 0.0 };
        let lb = if is_first { meta.left_border } else { 0.0 };
        let remaining = (flow.avail_w - flow.line_x - lp - lb).max(0.0);

        let mut split_byte =
            find_fit_prefix_end(rest, meta.style, remaining, prefer_space_break);

        if split_byte == 0 {
            if !flow.current_line.is_empty() {
                flow.flush_current_line();
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
        let rp = if is_last { meta.right_pad } else { 0.0 };
        let rb = if is_last { meta.right_border } else { 0.0 };
        let chunk_w = measure_text(chunk, meta.style);

        push_fragment(
            &mut flow.current_line,
            TextLineFragment {
                text: chunk.to_string(),
                node_id: Some(meta.node_id),
                style: meta.style,
                href: meta.href.clone(),
                span_id: meta.span_id,
                line_height: lh,
                text_width: chunk_w,
                left_pad: lp,
                left_border: lb,
                right_pad: rp,
                right_border: rb,
            },
        );
        flow.line_x += lp + lb + chunk_w + rp + rb;
        is_first = false;

        if !rest.is_empty() {
            flow.flush_current_line();
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
