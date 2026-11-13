//! Replaced/atomic layout for HTML form controls.
//!
//! Controls are laid out as deterministic atomic boxes in block, inline and flex
//! contexts. Their *values* live in the arena DOM (`value` attribute for inputs,
//! text content for `<textarea>`/`<button>`, attributes for checked/selected), so
//! the shell only owns transient focus/caret/selection state.

use droplet_dom::{Document, ElementData, NodeId};
use droplet_gfx::font::line_height_px;
use droplet_gfx::{Color, Rect};
use droplet_style::{Display, EdgeSizes, StyledNode};

use crate::box_model::{
    fixed_horizontal_margins, measure_text, resolve_height, resolve_max_width, resolve_width,
    BoxType, Dimensions, LayoutBox, LayoutCtx,
};

/// Kinds of interactive control rendered by Droplet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlKind {
    TextInput,
    PasswordInput,
    Checkbox,
    Radio,
    Submit,
    Button,
    Textarea,
    Select,
    Hidden,
}

impl ControlKind {
    pub fn is_text_like(self) -> bool {
        matches!(
            self,
            ControlKind::TextInput | ControlKind::PasswordInput | ControlKind::Textarea
        )
    }

    pub fn is_checkable(self) -> bool {
        matches!(self, ControlKind::Checkbox | ControlKind::Radio)
    }

    pub fn is_button_like(self) -> bool {
        matches!(self, ControlKind::Submit | ControlKind::Button)
    }
}

/// Deterministic, paint-ready snapshot of a form control.
#[derive(Debug, Clone, PartialEq)]
pub struct ControlBox {
    pub kind: ControlKind,
    /// Visible text (password inputs contain bullet glyphs, never the secret).
    pub text: String,
    pub placeholder: String,
    pub checked: bool,
    pub disabled: bool,
}

fn has_bool_attr(elem: &ElementData, name: &str) -> bool {
    match elem.attr(name) {
        None => false,
        Some(raw) => {
            let v = raw.trim();
            !(v.eq_ignore_ascii_case("false") || v == "0")
        }
    }
}

/// Returns the control kind for `node_id`, or `None` for non-control elements.
pub fn control_kind(doc: &Document, node_id: NodeId) -> Option<ControlKind> {
    let elem = doc.element(node_id)?;
    if elem.tag_name.eq_ignore_ascii_case("input") {
        let ty = elem.attr("type").unwrap_or("text").trim();
        return Some(if ty.eq_ignore_ascii_case("password") {
            ControlKind::PasswordInput
        } else if ty.eq_ignore_ascii_case("checkbox") {
            ControlKind::Checkbox
        } else if ty.eq_ignore_ascii_case("radio") {
            ControlKind::Radio
        } else if ty.eq_ignore_ascii_case("submit") {
            ControlKind::Submit
        } else if ty.eq_ignore_ascii_case("hidden") {
            ControlKind::Hidden
        } else {
            ControlKind::TextInput
        });
    }
    if elem.tag_name.eq_ignore_ascii_case("textarea") {
        return Some(ControlKind::Textarea);
    }
    if elem.tag_name.eq_ignore_ascii_case("button") {
        return Some(ControlKind::Button);
    }
    if elem.tag_name.eq_ignore_ascii_case("select") {
        return Some(ControlKind::Select);
    }
    None
}

pub(crate) fn is_control_element(doc: &Document, node_id: NodeId) -> bool {
    control_kind(doc, node_id).is_some()
}

/// Reads the raw editable value stored in the DOM.
pub fn control_value(doc: &Document, node_id: NodeId) -> String {
    let Some(elem) = doc.element(node_id) else {
        return String::new();
    };
    match control_kind(doc, node_id) {
        Some(ControlKind::TextInput | ControlKind::PasswordInput) => {
            elem.attr("value").unwrap_or("").to_string()
        }
        Some(ControlKind::Textarea) => doc.text_content(node_id),
        Some(ControlKind::Button) => doc.text_content(node_id),
        Some(ControlKind::Submit) => elem.attr("value").unwrap_or("Submit").to_string(),
        Some(ControlKind::Checkbox | ControlKind::Radio) => {
            elem.attr("value").unwrap_or("on").to_string()
        }
        Some(ControlKind::Select) | Some(ControlKind::Hidden) | None => String::new(),
    }
}

fn selected_option_id(doc: &Document, select_id: NodeId) -> Option<NodeId> {
    let children = &doc.get(select_id)?.children;
    let mut first = None;
    let mut selected = None;
    for &child in children {
        let Some(elem) = doc.element(child) else {
            continue;
        };
        if !elem.tag_name.eq_ignore_ascii_case("option") {
            continue;
        }
        if first.is_none() {
            first = Some(child);
        }
        if has_bool_attr(elem, "selected") {
            selected = Some(child);
            break;
        }
    }
    selected.or(first)
}

pub fn selected_option(doc: &Document, select_id: NodeId) -> Option<NodeId> {
    selected_option_id(doc, select_id)
}

fn select_display_text(doc: &Document, select_id: NodeId) -> String {
    let Some(option) = selected_option_id(doc, select_id) else {
        return String::new();
    };
    doc.text_content(option).trim().to_string()
}

/// Builds the paint-ready control snapshot for `node_id`.
pub fn control_snapshot(doc: &Document, node_id: NodeId) -> Option<ControlBox> {
    let kind = control_kind(doc, node_id)?;
    let elem = doc.element(node_id)?;
    let placeholder = elem.attr("placeholder").unwrap_or("").to_string();
    let checked = has_bool_attr(elem, "checked");
    let disabled = has_bool_attr(elem, "disabled");
    let text = match kind {
        ControlKind::PasswordInput => {
            let raw = elem.attr("value").unwrap_or("");
            raw.chars().map(|_| '\u{2022}').collect()
        }
        ControlKind::Select => select_display_text(doc, node_id),
        ControlKind::Hidden => String::new(),
        _ => control_value(doc, node_id),
    };
    Some(ControlBox {
        kind,
        text,
        placeholder,
        checked,
        disabled,
    })
}

pub(crate) fn effective_padding(style: droplet_style::ComputedStyle, kind: ControlKind) -> EdgeSizes {
    match kind {
        ControlKind::Checkbox | ControlKind::Radio | ControlKind::Hidden => EdgeSizes::default(),
        ControlKind::TextInput | ControlKind::PasswordInput => {
            if style.padding == EdgeSizes::default() {
                EdgeSizes {
                    top: 5.0,
                    right: 8.0,
                    bottom: 5.0,
                    left: 8.0,
                }
            } else {
                style.padding
            }
        }
        ControlKind::Submit => {
            if style.padding == EdgeSizes::default() {
                EdgeSizes {
                    top: 6.0,
                    right: 12.0,
                    bottom: 6.0,
                    left: 12.0,
                }
            } else {
                style.padding
            }
        }
        _ => style.padding,
    }
}

pub(crate) fn effective_border(style: droplet_style::ComputedStyle, _kind: ControlKind) -> EdgeSizes {
    if style.border != EdgeSizes::default() {
        return style.border;
    }
    EdgeSizes {
        top: 1.0,
        right: 1.0,
        bottom: 1.0,
        left: 1.0,
    }
}

/// Horizontal padding + border actually used by a control. This can differ from
/// the cascaded style for inputs whose UA defaults are expanded in layout.
pub(crate) fn control_extra_horizontal(doc: &Document, styled: &StyledNode) -> f32 {
    let Some(kind) = control_kind(doc, styled.node_id) else {
        return styled.style.padding.left
            + styled.style.padding.right
            + styled.style.border.left
            + styled.style.border.right;
    };
    let padding = effective_padding(styled.style, kind);
    let border = effective_border(styled.style, kind);
    padding.left + padding.right + border.left + border.right
}

fn intrinsic_content_size(
    styled: &StyledNode,
    kind: ControlKind,
    control: &ControlBox,
) -> (f32, f32) {
    let fs = styled.style.font_size;
    let line_h = line_height_px(fs);
    match kind {
        ControlKind::Checkbox | ControlKind::Radio | ControlKind::Hidden => (14.0, 14.0),
        ControlKind::TextInput | ControlKind::PasswordInput => (180.0, line_h),
        ControlKind::Textarea => (240.0, line_h * 3.0),
        ControlKind::Submit | ControlKind::Button => {
            let label = if control.text.is_empty() {
                "Button"
            } else {
                control.text.as_str()
            };
            ((measure_text(label, styled.style) + 4.0).max(40.0), line_h + 2.0)
        }
        ControlKind::Select => {
            let label_w = measure_text(control.text.as_str(), styled.style);
            ((label_w + 28.0).max(64.0), line_h + 2.0)
        }
    }
}

/// Intrinsic **content** width of a control for inline/flex measurement.
pub(crate) fn measure_control_content_width(
    ctx: &LayoutCtx<'_>,
    styled: &StyledNode,
    containing_width: f32,
) -> f32 {
    let Some(kind) = control_kind(ctx.doc, styled.node_id) else {
        return 0.0;
    };
    let Some(control) = control_snapshot(ctx.doc, styled.node_id) else {
        return 0.0;
    };
    if let Some(w) = resolve_width(styled.style, containing_width) {
        let max_w = resolve_max_width(styled.style, containing_width);
        return if let Some(m) = max_w { w.min(m) } else { w }.max(0.0);
    }
    let (intrinsic, _) = intrinsic_content_size(styled, kind, &control);
    let max_w = resolve_max_width(styled.style, containing_width);
    if let Some(m) = max_w {
        intrinsic.min(m).max(0.0)
    } else {
        intrinsic
    }
}

pub(crate) fn layout_control_box(
    ctx: &LayoutCtx<'_>,
    styled: &StyledNode,
    containing_rect: Rect,
    border_box_y: f32,
    containing_height: Option<f32>,
    forced_content_w: Option<f32>,
    forced_content_h: Option<f32>,
) -> LayoutBox {
    let kind = control_kind(ctx.doc, styled.node_id).unwrap_or(ControlKind::TextInput);
    let control = control_snapshot(ctx.doc, styled.node_id).unwrap_or(ControlBox {
        kind,
        text: String::new(),
        placeholder: String::new(),
        checked: false,
        disabled: false,
    });
    let (intrinsic_w, intrinsic_h) = intrinsic_content_size(styled, kind, &control);

    let mut padding = effective_padding(styled.style, kind);
    let mut border = effective_border(styled.style, kind);
    if kind == ControlKind::Hidden {
        padding = EdgeSizes::default();
        border = EdgeSizes::default();
    }

    let mut margin = styled.style.margin;
    let non_content = padding.left + padding.right + border.left + border.right;
    let explicit_w = forced_content_w.or_else(|| resolve_width(styled.style, containing_rect.width));
    let max_w = resolve_max_width(styled.style, containing_rect.width);
    let content_w = match explicit_w {
        Some(w) => {
            let w = if let Some(m) = max_w { w.min(m) } else { w };
            w.max(0.0)
        }
        None => {
            let w = if let Some(m) = max_w {
                intrinsic_w.min(m)
            } else {
                intrinsic_w
            };
            w.max(0.0)
        }
    };

    if styled.style.display == Display::Block || styled.style.display == Display::Flex {
        let (fixed_left, fixed_right) = fixed_horizontal_margins(styled.style);
        let underflow =
            containing_rect.width - non_content - content_w - (fixed_left + fixed_right);
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

    let content_h = forced_content_h
        .or_else(|| resolve_height(styled.style, containing_height))
        .unwrap_or(intrinsic_h)
        .max(0.0);
    let content_x = containing_rect.x + margin.left + border.left + padding.left;
    let content_y = border_box_y + border.top + padding.top;

    let mut style = styled.style;
    style.padding = padding;
    style.border = border;
    let children = Vec::new();
    if kind == ControlKind::Hidden {
        // Hidden inputs occupy no visual space and are skipped by focus traversal.
        style.background_color = None;
        style.border_color = Color::rgba(0, 0, 0, 0);
        style.border_radius = 0.0;
    } else if kind.is_checkable() {
        style.border_radius = if kind == ControlKind::Radio { 7.0 } else { 3.0 };
        if style.border_color.a == 0 {
            style.border_color = Color::rgb(148, 163, 184);
        }
        if style.background_color.is_none() {
            style.background_color = Some(Color::rgb(255, 255, 255));
        }
    } else if kind == ControlKind::Submit {
        if style.border_radius == 0.0 {
            style.border_radius = 6.0;
        }
        if style.background_color.is_none() {
            style.background_color = Some(Color::rgb(248, 250, 252));
        }
    } else {
        if matches!(kind, ControlKind::TextInput | ControlKind::PasswordInput)
            && style.border_radius == 0.0
        {
            style.border_radius = 4.0;
        }
        if style.background_color.is_none() {
            style.background_color = Some(Color::rgb(255, 255, 255));
        }
    }
    if style.border_color.a == 0 && kind != ControlKind::Hidden {
        style.border_color = Color::rgb(148, 163, 184);
    }

    let (width, height) = if kind == ControlKind::Hidden {
        (0.0, 0.0)
    } else {
        (content_w, content_h)
    };

    LayoutBox {
        node_id: Some(styled.node_id),
        dimensions: Dimensions {
            content: Rect {
                x: content_x,
                y: content_y,
                width,
                height,
            },
            padding,
            border,
            margin,
        },
        box_type: BoxType::Control(control),
        style,
        href: styled.href.clone(),
        children,
    }
}
