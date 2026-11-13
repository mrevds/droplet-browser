use droplet_dom::{Document, NodeKind};
use droplet_gfx::Rect;
use droplet_style::{
    AlignItems, Display, FlexBasis, FlexDirection, FlexWrap, JustifyContent, StyledNode,
};

use crate::box_model::{
    box_dispatch, compute_block_horizontal, find_last_baseline_offset_from_margin_top,
    fixed_horizontal_margins, resolve_height, resolve_max_width, resolve_width, shift_layout_box,
    BoxType, Dimensions, LayoutBox, LayoutCtx,
};
use crate::forms;
use crate::inline::{layout_inline_sequence, measure_intrinsic_content_width};
use crate::replaced::{is_img_element, resolve_image_content_size};

pub(crate) fn is_skipped_flex_child(doc: &Document, child: &StyledNode) -> bool {
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

struct LineSummary {
    indices: Vec<usize>,
    cross_size: f32,
    max_baseline: f32,
}

#[derive(Clone, Copy)]
struct FlexParams {
    content_x: f32,
    content_y: f32,
    content_w: f32,
    explicit_h: Option<f32>,
    is_row: bool,
    is_reverse: bool,
    gap: f32,
    align_items: AlignItems,
    justify_content: JustifyContent,
    container_main_avail: Option<f32>,
    container_cross_definite: Option<f32>,
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
    let mut laid = box_dispatch(
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

fn collect_flex_items<'a>(
    ctx: &LayoutCtx<'_>,
    styled: &'a StyledNode,
    params: FlexParams,
) -> Vec<FlexItemMeasure<'a>> {
    let mut items: Vec<FlexItemMeasure<'a>> = Vec::new();
    for child in &styled.children {
        if is_skipped_flex_child(ctx.doc, child) {
            continue;
        }
        let is_text_node = matches!(
            ctx.doc.get(child.node_id).map(|n| &n.kind),
            Some(NodeKind::Text(_))
        );
        let s = child.style;
        let (ml, mr) = fixed_horizontal_margins(s);
        let control_extra = forms::control_extra_horizontal(ctx.doc, child);

        let (main_pad_border, main_margin, cross_pad_border, cross_margin) = if params.is_row {
            (
                control_extra,
                ml + mr,
                s.padding.top + s.padding.bottom + s.border.top + s.border.bottom,
                s.margin.top + s.margin.bottom,
            )
        } else {
            (
                s.padding.top + s.padding.bottom + s.border.top + s.border.bottom,
                s.margin.top + s.margin.bottom,
                control_extra,
                ml + mr,
            )
        };

        // Pass 1: measure base main size
        let basis_main = match s.flex_basis {
            FlexBasis::Px(px) => Some(px.max(0.0)),
            FlexBasis::Percent(pct) => {
                params.container_main_avail.map(|avail| (avail * (pct / 100.0)).max(0.0))
            }
            FlexBasis::Auto => None,
        };

        let base_content_main = if let Some(b) = basis_main {
            b
        } else if params.is_row {
            if let Some(w) = resolve_width(s, params.content_w) {
                let mw = resolve_max_width(s, params.content_w);
                if let Some(m) = mw { w.min(m) } else { w }.max(0.0)
            } else if is_img_element(ctx.doc, child.node_id) {
                resolve_image_content_size(
                    ctx,
                    child,
                    params.content_w,
                    params.explicit_h,
                    None,
                    None,
                )
                .width
            } else {
                let avail_c = (params.content_w - main_pad_border - main_margin).max(0.0);
                let (_, max_w) = measure_intrinsic_content_width(ctx, child, avail_c);
                let mut w = max_w.min(avail_c);
                if let Some(mw) = resolve_max_width(s, params.content_w) {
                    w = w.min(mw);
                }
                w.max(0.0)
            }
        } else if let Some(h) = resolve_height(s, params.explicit_h) {
            h.max(0.0)
        } else {
            let cross_w = compute_column_item_cross_width(
                ctx,
                child,
                params.content_w,
                cross_pad_border,
                cross_margin,
                params.align_items,
            );
            let tentative = layout_flex_item(
                ctx,
                child,
                is_text_node,
                params.content_w,
                params.explicit_h,
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
            auto_main_start: params.is_row && s.margin_left_auto,
            auto_main_end: params.is_row && s.margin_right_auto,
            layout: None,
            outer_main: hypothetical_outer_main,
            outer_cross: 0.0,
            baseline_offset: 0.0,
        });
    }
    items
}

fn break_flex_lines(
    items: &[FlexItemMeasure<'_>],
    wraps: bool,
    params: FlexParams,
) -> Vec<Vec<usize>> {
    let mut lines: Vec<Vec<usize>> = Vec::new();
    if !wraps || params.container_main_avail.is_none() {
        lines.push((0..items.len()).collect());
        return lines;
    }

    let avail_main = params.container_main_avail.unwrap_or(params.content_w);
    let mut cur_line: Vec<usize> = Vec::new();
    let mut cur_used = 0.0_f32;
    for (idx, it) in items.iter().enumerate() {
        let added = if cur_line.is_empty() {
            it.hypothetical_outer_main
        } else {
            params.gap + it.hypothetical_outer_main
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
    lines
}

fn resolve_line_flex_sizes(
    items: &mut [FlexItemMeasure<'_>],
    line_indices: &[usize],
    params: FlexParams,
) {
    let n = line_indices.len();
    let total_gaps = if n > 1 {
        params.gap * (n - 1) as f32
    } else {
        0.0
    };
    let sum_base_outer: f32 = line_indices
        .iter()
        .map(|&i| items[i].hypothetical_outer_main)
        .sum();
    let avail_main = params
        .container_main_avail
        .unwrap_or(sum_base_outer + total_gaps);
    let free_space = avail_main - (sum_base_outer + total_gaps);

    if free_space > 0.001 {
        let total_grow: f32 = line_indices.iter().map(|&i| items[i].flex_grow).sum();
        if total_grow > 0.0 {
            let denom = total_grow.max(1.0);
            for &i in line_indices {
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
                if frozen[pos] || items[i].flex_shrink <= 0.0 || items[i].final_content_main <= 0.0
                {
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
}

fn layout_flex_line_items(
    ctx: &LayoutCtx<'_>,
    items: &mut [FlexItemMeasure<'_>],
    line_indices: Vec<usize>,
    params: FlexParams,
    single_line: bool,
) -> LineSummary {
    let mut line_cross = 0.0_f32;
    let mut max_baseline = 0.0_f32;

    for &i in &line_indices {
        let final_main = items[i].final_content_main;
        let laid = if params.is_row {
            layout_flex_item(
                ctx,
                items[i].styled,
                items[i].is_text_node,
                params.content_w,
                params.explicit_h,
                Some(final_main),
                None,
            )
        } else {
            let cross_w = compute_column_item_cross_width(
                ctx,
                items[i].styled,
                params.content_w,
                items[i].cross_pad_border,
                items[i].cross_margin,
                params.align_items,
            );
            layout_flex_item(
                ctx,
                items[i].styled,
                items[i].is_text_node,
                params.content_w,
                params.explicit_h,
                Some(cross_w),
                Some(final_main),
            )
        };

        let mb = laid.dimensions.margin_box();
        let (outer_main, outer_cross) = if params.is_row {
            (mb.width, mb.height)
        } else {
            (mb.height, mb.width)
        };
        let baseline = if params.is_row {
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
        if let Some(def_cross) = params.container_cross_definite {
            line_cross = line_cross.max(def_cross);
        }
    }

    if params.is_row && params.align_items == AlignItems::Stretch {
        for &i in &line_indices {
            let has_explicit_h =
                resolve_height(items[i].styled.style, params.explicit_h).is_some();
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

    LineSummary {
        indices: line_indices,
        cross_size: line_cross,
        max_baseline,
    }
}

fn position_flex_lines(
    items: &mut [FlexItemMeasure<'_>],
    summaries: Vec<LineSummary>,
    params: FlexParams,
) -> (Vec<LayoutBox>, f32) {
    let mut final_children: Vec<LayoutBox> = Vec::with_capacity(items.len());
    let mut cross_cursor = 0.0_f32;
    let total_lines = summaries.len();

    for (line_idx, summary) in summaries.into_iter().enumerate() {
        let n = summary.indices.len();
        let total_gaps = if n > 1 {
            params.gap * (n - 1) as f32
        } else {
            0.0
        };
        let sum_outer_main: f32 = summary.indices.iter().map(|&i| items[i].outer_main).sum();
        let avail_main = params
            .container_main_avail
            .unwrap_or(sum_outer_main + total_gaps);
        let raw_leftover = avail_main - (sum_outer_main + total_gaps);

        let auto_margin_count: usize = summary
            .indices
            .iter()
            .map(|&i| usize::from(items[i].auto_main_start) + usize::from(items[i].auto_main_end))
            .sum();

        let (start_offset, extra_between, auto_margin_unit) =
            if params.is_row && auto_margin_count > 0 && raw_leftover > 0.0 {
                (0.0, 0.0, raw_leftover / auto_margin_count as f32)
            } else {
                let pos_leftover = raw_leftover.max(0.0);
                let (so, eb) = match params.justify_content {
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
            let visual_main = if params.is_reverse {
                avail_main - (main_cursor + item_outer_main)
            } else {
                main_cursor
            };

            let cross_offset = match params.align_items {
                AlignItems::Stretch | AlignItems::FlexStart => 0.0,
                AlignItems::FlexEnd => (summary.cross_size - items[i].outer_cross).max(0.0),
                AlignItems::Center => (summary.cross_size - items[i].outer_cross).max(0.0) * 0.5,
                AlignItems::Baseline => {
                    if params.is_row {
                        (summary.max_baseline - items[i].baseline_offset).max(0.0)
                    } else {
                        0.0
                    }
                }
            };

            let (target_mb_x, target_mb_y) = if params.is_row {
                (
                    params.content_x + visual_main,
                    params.content_y + cross_cursor + cross_offset,
                )
            } else {
                (
                    params.content_x + cross_cursor + cross_offset,
                    params.content_y + visual_main,
                )
            };

            if let Some(mut laid) = items[i].layout.take() {
                let mb = laid.dimensions.margin_box();
                shift_layout_box(&mut laid, target_mb_x - mb.x, target_mb_y - mb.y);
                final_children.push(laid);
            }

            main_cursor += item_outer_main + trail_auto;
            if pos + 1 < n {
                main_cursor += params.gap + extra_between;
            }
        }

        cross_cursor += summary.cross_size;
        if line_idx + 1 < total_lines {
            cross_cursor += params.gap;
        }
    }

    (final_children, cross_cursor)
}

pub(crate) fn layout_flex(
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

    let params = FlexParams {
        content_x,
        content_y,
        content_w,
        explicit_h: explicit_container_h,
        is_row,
        is_reverse,
        gap: styled.style.gap.max(0.0),
        align_items: styled.style.align_items,
        justify_content: styled.style.justify_content,
        container_main_avail: if is_row {
            Some(content_w)
        } else {
            explicit_container_h
        },
        container_cross_definite: if is_row {
            explicit_container_h
        } else {
            Some(content_w)
        },
    };

    let mut items = collect_flex_items(ctx, styled, params);

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

    let wraps = styled.style.flex_wrap != FlexWrap::NoWrap;
    let lines = break_flex_lines(&items, wraps, params);
    let single_line = lines.len() == 1;
    let mut summaries: Vec<LineSummary> = Vec::with_capacity(lines.len());

    for line_indices in lines {
        resolve_line_flex_sizes(&mut items, &line_indices, params);
        summaries.push(layout_flex_line_items(
            ctx,
            &mut items,
            line_indices,
            params,
            single_line,
        ));
    }

    if styled.style.flex_wrap == FlexWrap::WrapReverse {
        summaries.reverse();
    }

    let (final_children, cross_cursor) = position_flex_lines(&mut items, summaries, params);

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
        return resolve_image_content_size(ctx, child, container_w, None, None, None).width;
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
