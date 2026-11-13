//! Form-control painting and text-selection overlays.
//!
//! Layout produces atomic [`droplet_layout::ControlBox`] boxes for form controls.
//! This module turns those snapshots (plus transient focus/caret/selection state)
//! into ordinary display commands, so the software rasterizer needs no new
//! primitives and existing display-list tests keep their exact command shape.

use droplet_gfx::font;
use droplet_gfx::{Color, Rect};
use droplet_layout::{BoxType, ControlBox, ControlKind, LayoutBox};
use droplet_style::EdgeSizes;

use crate::display_list::{DisplayCommand, DisplayList};

/// Arena DOM node identifier (kept local so paint needs no DOM dependency).
pub type NodeId = usize;

/// Byte offset into the rendered text of a text box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextPosition {
    pub node: NodeId,
    pub offset: usize,
}

/// A drag/select-all range over rendered page text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageSelection {
    pub anchor: TextPosition,
    pub focus: TextPosition,
}

/// Caret and selection state for the currently focused form control.
///
/// Offsets are **character** indices into the control's visible text. Character
/// offsets keep password masking simple: the visible bullets have a different
/// byte length than the stored secrets they replace.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ControlVisual {
    pub caret_chars: Option<usize>,
    pub selection_chars: Option<(usize, usize)>,
}

/// Transient per-tab state consumed while building a display list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FormVisualState {
    pub focused: Option<NodeId>,
    pub control: ControlVisual,
    pub page_selection: Option<PageSelection>,
}

/// One painted text box, flattened in display-list (pre-order) order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextEntry {
    pub node: NodeId,
    pub text: String,
}

pub fn collect_text_entries(root: &LayoutBox) -> Vec<TextEntry> {
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if let BoxType::Text { lines, .. } = &node.box_type {
            if let Some(id) = node.node_id {
                out.push(TextEntry {
                    node: id,
                    text: lines.join(""),
                });
            }
        }
        for child in node.children.iter().rev() {
            stack.push(child);
        }
    }
    out
}

/// Returns `(entry_index, start_byte, end_byte)` for every text entry covered by
/// `selection`. Offsets are clamped to valid UTF-8 boundaries by callers of the
/// painting code before slicing.
pub fn selection_spans(
    entries: &[TextEntry],
    selection: &PageSelection,
) -> Vec<(usize, usize, usize)> {
    let anchor_idx = entries
        .iter()
        .position(|e| e.node == selection.anchor.node);
    let focus_idx = entries.iter().position(|e| e.node == selection.focus.node);
    let (Some(a_idx), Some(f_idx)) = (anchor_idx, focus_idx) else {
        return Vec::new();
    };
    let (lo_idx, hi_idx, lo_off, hi_off) =
        if a_idx < f_idx || (a_idx == f_idx && selection.anchor.offset <= selection.focus.offset) {
            (
                a_idx,
                f_idx,
                selection.anchor.offset,
                selection.focus.offset,
            )
        } else {
            (
                f_idx,
                a_idx,
                selection.focus.offset,
                selection.anchor.offset,
            )
        };

    let mut spans = Vec::new();
    for (idx, entry) in entries.iter().enumerate().take(hi_idx + 1).skip(lo_idx) {
        let len = entry.text.len();
        let start = if idx == lo_idx { lo_off.min(len) } else { 0 };
        let end = if idx == hi_idx { hi_off.min(len) } else { len };
        let start = floor_char_boundary(&entry.text, start);
        let end = ceil_char_boundary(&entry.text, end).max(start);
        if end > start {
            spans.push((idx, start, end));
        }
    }
    spans
}

/// Extracts the selected rendered text, joining separate text boxes with a
/// newline so paragraphs copied from a page remain readable.
pub fn selection_text(entries: &[TextEntry], selection: &PageSelection) -> String {
    let spans = selection_spans(entries, selection);
    let mut parts: Vec<String> = Vec::new();
    for (idx, start, end) in spans {
        let text = &entries[idx].text[start..end];
        if !text.is_empty() {
            parts.push(text.to_string());
        }
    }
    parts.join("\n")
}

fn floor_char_boundary(text: &str, mut idx: usize) -> usize {
    idx = idx.min(text.len());
    while idx > 0 && !text.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

fn ceil_char_boundary(text: &str, mut idx: usize) -> usize {
    idx = idx.min(text.len());
    while idx < text.len() && !text.is_char_boundary(idx) {
        idx += 1;
    }
    idx
}

fn byte_at_char_index(text: &str, chars: usize) -> usize {
    text.char_indices()
        .nth(chars)
        .map(|(i, _)| i)
        .unwrap_or(text.len())
}

fn char_index_at_byte(text: &str, byte: usize) -> usize {
    let byte = floor_char_boundary(text, byte);
    text[..byte].chars().count()
}

/// Converts a caret byte offset into the character offset used by paint visuals.
pub fn caret_chars(text: &str, byte: usize) -> usize {
    char_index_at_byte(text, byte)
}

/// Converts a byte selection range into character offsets used by paint visuals.
pub fn selection_chars(text: &str, start: usize, end: usize) -> (usize, usize) {
    (
        char_index_at_byte(text, start),
        char_index_at_byte(text, end),
    )
}

fn truncate_to_width(text: &str, style: droplet_style::ComputedStyle, max_w: f32) -> &str {
    if max_w <= 0.0 {
        return "";
    }
    if font::measure_text_px(text, style.font_size, style.bold, style.monospace) <= max_w {
        return text;
    }
    let mut last = 0usize;
    for (idx, ch) in text.char_indices() {
        let end = idx + ch.len_utf8();
        if font::measure_text_px(&text[..end], style.font_size, style.bold, style.monospace) <= max_w
        {
            last = end;
        } else {
            break;
        }
    }
    &text[..last]
}

fn text_width(text: &str, box_style: droplet_style::ComputedStyle) -> f32 {
    font::measure_text_px(
        text,
        box_style.font_size,
        box_style.bold,
        box_style.monospace,
    )
}

fn emit_text(
    list: &mut DisplayList,
    x: f32,
    y: f32,
    text: &str,
    style: droplet_style::ComputedStyle,
    color: Color,
) {
    if text.is_empty() {
        return;
    }
    list.push(DisplayCommand::Text {
        x,
        y,
        text: text.to_string(),
        color,
        font_size: style.font_size,
        scale: if style.font_size >= 22.0 { 2 } else { 1 },
        bold: style.bold,
        monospace: style.monospace,
        underline: false,
        width: text_width(text, style),
    });
}

fn emit_selection_rect(list: &mut DisplayList, rect: Rect) {
    if rect.width <= 0.0 || rect.height <= 0.0 {
        return;
    }
    list.push(DisplayCommand::SolidRect {
        rect,
        color: Color::rgba(37, 99, 235, 74),
        radius: 0.0,
    });
}

/// Emits page-text selection highlights for one text box (before its glyphs),
/// given a precomputed byte range `[start, end)` inside its concatenated lines.
pub(crate) fn emit_text_selection_range(
    layout_box: &LayoutBox,
    start: usize,
    end: usize,
    list: &mut DisplayList,
) {
    if end <= start {
        return;
    }
    let BoxType::Text {
        lines,
        line_height,
        font_size,
        monospace,
        ..
    } = &layout_box.box_type
    else {
        return;
    };
    let mut cumulative = 0usize;
    let base_x = layout_box.dimensions.content.x;
    let base_y = layout_box.dimensions.content.y;
    for (line_idx, line) in lines.iter().enumerate() {
        let line_start = cumulative;
        let line_end = cumulative + line.len();
        cumulative = line_end;
        let sel_start = floor_char_boundary(line, start.max(line_start).min(line_end) - line_start);
        let sel_end = ceil_char_boundary(line, end.min(line_end).saturating_sub(line_start));
        if sel_end <= sel_start {
            continue;
        }
        let prefix = &line[..sel_start];
        let selected = &line[sel_start..sel_end];
        let x0 = base_x + font::measure_text_px(prefix, *font_size, layout_box.style.bold, *monospace);
        let x1 = x0
            + font::measure_text_px(selected, *font_size, layout_box.style.bold, *monospace);
        emit_selection_rect(
            list,
            Rect {
                x: x0,
                y: base_y + line_idx as f32 * *line_height,
                width: x1 - x0,
                height: *line_height,
            },
        );
    }
}

fn control_text_lines(control: &ControlBox, placeholder: bool) -> Vec<String> {
    let text = if control.text.is_empty() && placeholder {
        control.placeholder.as_str()
    } else {
        control.text.as_str()
    };
    if control.kind == ControlKind::Textarea {
        text.split('\n').map(str::to_string).collect()
    } else {
        vec![text.replace(['\n', '\r'], " ")]
    }
}

fn emit_check_mark(
    list: &mut DisplayList,
    kind: ControlKind,
    content: Rect,
    disabled: bool,
) {
    let color = if disabled {
        Color::rgb(148, 163, 184)
    } else {
        Color::rgb(37, 99, 235)
    };
    let size = (content.width.min(content.height) * 0.5).max(4.0);
    if kind == ControlKind::Radio {
        let d = size.min(content.width).min(content.height);
        list.push(DisplayCommand::SolidRect {
            rect: Rect {
                x: content.x + (content.width - d) * 0.5,
                y: content.y + (content.height - d) * 0.5,
                width: d,
                height: d,
            },
            color,
            radius: d * 0.5,
        });
    } else {
        let inset = ((content.width.min(content.height) - size) * 0.5).max(0.0);
        list.push(DisplayCommand::SolidRect {
            rect: Rect {
                x: content.x + inset,
                y: content.y + inset,
                width: (content.width - inset * 2.0).max(0.0),
                height: (content.height - inset * 2.0).max(0.0),
            },
            color,
            radius: 2.0,
        });
    }
}

/// Emits the focus ring before the normal background/border paint so the ring
/// stays visible even when an author stylesheet overrides `border-color`.
pub(crate) fn emit_focus_ring(
    layout_box: &LayoutBox,
    visual: &FormVisualState,
    list: &mut DisplayList,
) {
    if visual.focused != layout_box.node_id {
        return;
    }
    let bb = layout_box.dimensions.border_box();
    let radius = layout_box.style.border_radius.max(0.0) + 2.0;
    list.push(DisplayCommand::SolidRect {
        rect: Rect {
            x: bb.x - 2.0,
            y: bb.y - 2.0,
            width: bb.width + 4.0,
            height: bb.height + 4.0,
        },
        color: Color::rgb(37, 99, 235),
        radius,
    });
}

/// Emits control chrome contents (value/placeholder/caret/selection/checked
/// mark/label) for one atomic control box.
pub(crate) fn emit_control(
    layout_box: &LayoutBox,
    control: &ControlBox,
    visual: &FormVisualState,
    list: &mut DisplayList,
) {
    let content = layout_box.dimensions.content;
    let style = layout_box.style;
    let focused = visual.focused == layout_box.node_id;
    let muted = if control.disabled {
        Color::rgb(148, 163, 184)
    } else {
        Color::rgb(100, 116, 139)
    };
    let text_color = if control.disabled {
        Color::rgb(148, 163, 184)
    } else {
        style.color
    };

    match control.kind {
        ControlKind::Hidden => {}
        ControlKind::Checkbox | ControlKind::Radio => {
            if control.checked {
                emit_check_mark(list, control.kind, content, control.disabled);
            }
        }
        ControlKind::TextInput | ControlKind::PasswordInput => {
            let lines = control_text_lines(control, true);
            let raw = lines.first().cloned().unwrap_or_default();
            let placeholder = control.text.is_empty() && !control.placeholder.is_empty();
            let line_height = font::line_height_px(style.font_size);
            let visible = truncate_to_width(&raw, style, content.width);
            let y = content.y + ((content.height - line_height) * 0.5).max(0.0);

            if focused {
                emit_control_selection(
                    list,
                    content,
                    style,
                    &raw,
                    visual.control.selection_chars,
                    y,
                    line_height,
                    visible.len(),
                );
            }

            emit_text(
                list,
                content.x,
                y,
                visible,
                style,
                if placeholder { muted } else { text_color },
            );

            if focused {
                if let Some(chars) = visual.control.caret_chars {
                    let byte = byte_at_char_index(&raw, chars).min(visible.len());
                    let x = content.x + text_width(&raw[..byte], style);
                    let x = x.min(content.x + content.width - 1.0).max(content.x);
                    list.push(DisplayCommand::SolidRect {
                        rect: Rect {
                            x,
                            y: content.y + ((content.height - line_height) * 0.5).max(0.0),
                            width: 1.0,
                            height: line_height.min(content.height),
                        },
                        color: Color::rgb(15, 23, 42),
                        radius: 0.0,
                    });
                }
            }
        }
        ControlKind::Textarea => {
            let lines = control_text_lines(control, true);
            let placeholder = control.text.is_empty() && !control.placeholder.is_empty();
            let line_height = font::line_height_px(style.font_size);
            let rows = ((content.height / line_height).floor() as usize).max(1);

            if focused {
                emit_control_selection_multiline(
                    list,
                    content,
                    style,
                    &control.text,
                    visual.control.selection_chars,
                    line_height,
                    rows,
                );
            }

            for (idx, line) in lines.iter().take(rows).enumerate() {
                let visible = truncate_to_width(line, style, content.width);
                emit_text(
                    list,
                    content.x,
                    content.y + idx as f32 * line_height,
                    visible,
                    style,
                    if placeholder { muted } else { text_color },
                );
            }

            if focused {
                if let Some(chars) = visual.control.caret_chars {
                    let (line_idx, col) = line_col_at_char(&control.text, chars);
                    if line_idx < rows {
                        let line = control.text.split('\n').nth(line_idx).unwrap_or("");
                        let byte = byte_at_char_index(line, col);
                        let x = content.x + text_width(&line[..byte], style);
                        list.push(DisplayCommand::SolidRect {
                            rect: Rect {
                                x: x.min(content.x + content.width - 1.0).max(content.x),
                                y: content.y + line_idx as f32 * line_height,
                                width: 1.0,
                                height: line_height,
                            },
                            color: Color::rgb(15, 23, 42),
                            radius: 0.0,
                        });
                    }
                }
            }
        }
        ControlKind::Submit | ControlKind::Button | ControlKind::Select => {
            let label = if control.text.is_empty() {
                if control.kind == ControlKind::Submit {
                    "Submit"
                } else {
                    ""
                }
            } else {
                control.text.as_str()
            };
            let line_height = font::line_height_px(style.font_size);
            let y = content.y + ((content.height - line_height) * 0.5).max(0.0);
            let label_w = text_width(label, style);
            let visible = truncate_to_width(label, style, content.width);
            let x = if control.kind == ControlKind::Select {
                content.x
            } else {
                content.x + ((content.width - label_w) * 0.5).max(0.0)
            };
            let color = if control.disabled { muted } else { text_color };
            emit_text(list, x, y, visible, style, color);
            if control.kind == ControlKind::Select {
                let arrow = "\u{25BE}";
                let w = text_width(arrow, style);
                if content.width > w + 4.0 {
                    emit_text(
                        list,
                        content.x + content.width - w - 2.0,
                        y,
                        arrow,
                        style,
                        muted,
                    );
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_control_selection(
    list: &mut DisplayList,
    content: Rect,
    style: droplet_style::ComputedStyle,
    text: &str,
    selection: Option<(usize, usize)>,
    y: f32,
    line_height: f32,
    visible_len: usize,
) {
    let Some((start_chars, end_chars)) = selection else {
        return;
    };
    if end_chars <= start_chars {
        return;
    }
    let total_chars = text.chars().count();
    let start_chars = start_chars.min(total_chars);
    let end_chars = end_chars.min(total_chars);
    let start_byte = byte_at_char_index(text, start_chars);
    let end_byte = byte_at_char_index(text, end_chars).min(visible_len);
    if end_byte <= start_byte {
        return;
    }
    let x0 = content.x + text_width(&text[..start_byte], style);
    let x1 = content.x + text_width(&text[..end_byte], style);
    emit_selection_rect(
        list,
        Rect {
            x: x0.min(content.x + content.width),
            y,
            width: (x1 - x0).min(content.width),
            height: line_height.min(content.height),
        },
    );
}

#[allow(clippy::too_many_arguments)]
fn emit_control_selection_multiline(
    list: &mut DisplayList,
    content: Rect,
    style: droplet_style::ComputedStyle,
    text: &str,
    selection: Option<(usize, usize)>,
    line_height: f32,
    rows: usize,
) {
    let Some((start_chars, end_chars)) = selection else {
        return;
    };
    if end_chars <= start_chars {
        return;
    }
    let total_chars = text.chars().count();
    let start_chars = start_chars.min(total_chars);
    let end_chars = end_chars.min(total_chars);
    let mut running = 0usize;
    for (line_idx, line) in text.split('\n').enumerate().take(rows) {
        let line_chars = line.chars().count();
        let line_start = running;
        let line_end = running + line_chars;
        running = line_end + 1;
        let start = start_chars.max(line_start).min(line_end) - line_start;
        let end = end_chars.min(line_end).saturating_sub(line_start);
        if end <= start {
            continue;
        }
        let (start_byte, end_byte) = (
            byte_at_char_index(line, start),
            byte_at_char_index(line, end),
        );
        let x0 = content.x + text_width(&line[..start_byte], style);
        let x1 = content.x + text_width(&line[..end_byte], style);
        emit_selection_rect(
            list,
            Rect {
                x: x0,
                y: content.y + line_idx as f32 * line_height,
                width: x1 - x0,
                height: line_height,
            },
        );
    }
}

/// Splits a character offset into `(line_index, column_in_chars)`.
pub fn line_col_at_char(text: &str, chars: usize) -> (usize, usize) {
    if chars == 0 {
        return (0, 0);
    }
    let mut line_idx = 0usize;
    let mut col = 0usize;
    for ch in text.chars().take(chars) {
        if ch == '\n' {
            line_idx += 1;
            col = 0;
        } else {
            col += 1;
        }
    }
    (line_idx, col)
}

/// Horizontal padding/border of a control as painted. Kept here so selection
/// highlight offsets and painted glyph positions share one source of truth.
pub fn control_content_inset(layout_box: &LayoutBox) -> EdgeSizes {
    layout_box.dimensions.padding
}
