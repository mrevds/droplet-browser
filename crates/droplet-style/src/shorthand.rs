//! Box-model and font property parsing: `box-sizing`, `min-*`/`max-height`, insets, and the
//! `padding` / `border*` / `background` / `font` shorthands.
//!
//! Every shorthand is all-or-nothing: an invalid component (unknown keyword, duplicate part,
//! wrong count, negative padding) drops the whole declaration, as CSS requires.

use std::borrow::Cow;

use crate::cascade::{is_monospace_font, resolve_length, LengthEnv};
use crate::computed::{BoxSizing, ComputedStyle, EdgeSizes, LengthPct};
use crate::css::{self, Color, Unit, Value};

pub(crate) const SIDE_TOP: u8 = 1;
pub(crate) const SIDE_RIGHT: u8 = 2;
pub(crate) const SIDE_BOTTOM: u8 = 4;
pub(crate) const SIDE_LEFT: u8 = 8;
pub(crate) const ALL_SIDES: u8 = SIDE_TOP | SIDE_RIGHT | SIDE_BOTTOM | SIDE_LEFT;

/// Per-element cascade scratch for borders; never stored in `ComputedStyle`.
pub(crate) struct BorderState {
    /// `border-color` was given explicitly (otherwise it follows `color`).
    pub explicit_color: bool,
    /// Sides whose `border-style` is visible (not `none`/`hidden`), one bit per side.
    pub styled_sides: u8,
}

impl BorderState {
    pub(crate) fn new() -> Self {
        Self {
            explicit_color: false,
            styled_sides: 0,
        }
    }

    /// A side with `border-style: none` has a computed width of 0 whatever `border-width` says.
    pub(crate) fn finish(&self, style: &mut ComputedStyle) {
        if self.styled_sides & SIDE_TOP == 0 {
            style.border.top = 0.0;
        }
        if self.styled_sides & SIDE_RIGHT == 0 {
            style.border.right = 0.0;
        }
        if self.styled_sides & SIDE_BOTTOM == 0 {
            style.border.bottom = 0.0;
        }
        if self.styled_sides & SIDE_LEFT == 0 {
            style.border.left = 0.0;
        }
        if !self.explicit_color {
            style.border_color = style.color;
        }
    }
}

/// Until custom properties are resolved (DROP-121), a `var(--x, <fallback>)` component is
/// replaced by its fallback, and a `var()` without one is dropped, so a shorthand such as
/// `border: 1px solid var(--c, #a2a9b1)` stays valid instead of being thrown away.
pub(crate) fn substitute_var_fallbacks(values: &[Value]) -> Cow<'_, [Value]> {
    let is_var = |v: &Value| matches!(v, Value::Keyword(k) if k.starts_with("var("));
    if !values.iter().any(is_var) {
        return Cow::Borrowed(values);
    }
    let mut out = Vec::with_capacity(values.len());
    for v in values {
        match v {
            Value::Keyword(k) if k.starts_with("var(") => {
                let inner = k[4..].strip_suffix(')').unwrap_or(&k[4..]);
                if let Some((_, fallback)) = inner.split_once(',') {
                    out.extend(css::parse_values(fallback));
                }
            }
            other => out.push(other.clone()),
        }
    }
    Cow::Owned(out)
}

pub(crate) fn expand_four_sides<T: Copy>(values: &[T]) -> Option<(T, T, T, T)> {
    match values.len() {
        1 => Some((values[0], values[0], values[0], values[0])),
        2 => Some((values[0], values[1], values[0], values[1])),
        3 => Some((values[0], values[1], values[2], values[1])),
        4 => Some((values[0], values[1], values[2], values[3])),
        _ => None,
    }
}

fn set_sides(edge: &mut EdgeSizes, mask: u8, w: f32) {
    if mask & SIDE_TOP != 0 {
        edge.top = w;
    }
    if mask & SIDE_RIGHT != 0 {
        edge.right = w;
    }
    if mask & SIDE_BOTTOM != 0 {
        edge.bottom = w;
    }
    if mask & SIDE_LEFT != 0 {
        edge.left = w;
    }
}

/// `padding: <length>{1,4}`; negative values make the declaration invalid.
pub(crate) fn apply_padding(edge: &mut EdgeSizes, values: &[Value], env: &LengthEnv<'_>) {
    let values = &*substitute_var_fallbacks(values);
    let parsed: Option<Vec<f32>> = values
        .iter()
        .map(|v| resolve_length(v, env).filter(|n| *n >= 0.0))
        .collect();
    if let Some((t, r, b, l)) = parsed.as_deref().and_then(expand_four_sides) {
        *edge = EdgeSizes {
            top: t,
            right: r,
            bottom: b,
            left: l,
        };
    }
}

pub(crate) fn apply_padding_side(side: &mut f32, values: &[Value], env: &LengthEnv<'_>) {
    if let [v] = values {
        if let Some(n) = resolve_length(v, env).filter(|n| *n >= 0.0) {
            *side = n;
        }
    }
}

/// `<line-width>`: a non-negative length or `thin`/`medium`/`thick`.
pub(crate) fn border_width_value(val: &Value, env: &LengthEnv<'_>) -> Option<f32> {
    if let Value::Keyword(kw) = val {
        return match kw.as_str() {
            "thin" => Some(1.0),
            "medium" => Some(3.0),
            "thick" => Some(5.0),
            _ => None,
        };
    }
    resolve_length(val, env).filter(|n| *n >= 0.0)
}

/// `<line-style>`: `Some(true)` for a visible style, `Some(false)` for `none`/`hidden`.
fn border_style_value(val: &Value) -> Option<bool> {
    let Value::Keyword(kw) = val else {
        return None;
    };
    match kw.as_str() {
        "none" | "hidden" => Some(false),
        "solid" | "dashed" | "dotted" | "double" | "groove" | "ridge" | "inset" | "outset" => {
            Some(true)
        }
        _ => None,
    }
}

fn border_color_value(val: &Value) -> Option<Option<Color>> {
    match val {
        Value::Color(c) => Some(Some(*c)),
        Value::Keyword(k) if k == "currentcolor" => Some(None),
        _ => None,
    }
}

fn set_styled(state: &mut BorderState, mask: u8, visible: bool) {
    if visible {
        state.styled_sides |= mask;
    } else {
        state.styled_sides &= !mask;
    }
}

/// `border` (mask = all sides) and `border-<side>`: `<line-width> || <line-style> || <color>`.
/// Omitted parts reset to their initial values (`medium`, `none`, `currentcolor`).
pub(crate) fn apply_border(
    style: &mut ComputedStyle,
    state: &mut BorderState,
    mask: u8,
    values: &[Value],
    env: &LengthEnv<'_>,
) {
    let values = &*substitute_var_fallbacks(values);
    let mut width: Option<f32> = None;
    let mut visible: Option<bool> = None;
    let mut color: Option<Option<Color>> = None;
    if values.is_empty() || values.len() > 3 {
        return;
    }
    for val in values {
        if let Some(v) = border_style_value(val) {
            if visible.replace(v).is_some() {
                return;
            }
        } else if let Some(w) = border_width_value(val, env) {
            if width.replace(w).is_some() {
                return;
            }
        } else if let Some(c) = border_color_value(val) {
            if color.replace(c).is_some() {
                return;
            }
        } else {
            return;
        }
    }
    set_sides(&mut style.border, mask, width.unwrap_or(3.0));
    set_styled(state, mask, visible.unwrap_or(false));
    // One border color is painted for all sides: a side shorthand only changes it when it names
    // a color; the full `border` shorthand also resets it to `currentcolor`.
    match color {
        Some(Some(c)) => {
            style.border_color = c;
            state.explicit_color = true;
        }
        Some(None) => state.explicit_color = false,
        None if mask == ALL_SIDES => state.explicit_color = false,
        None => {}
    }
}

/// `border-width: <line-width>{1,4}`.
pub(crate) fn apply_border_width(edge: &mut EdgeSizes, values: &[Value], env: &LengthEnv<'_>) {
    let values = &*substitute_var_fallbacks(values);
    let parsed: Option<Vec<f32>> = values.iter().map(|v| border_width_value(v, env)).collect();
    if let Some((t, r, b, l)) = parsed.as_deref().and_then(expand_four_sides) {
        *edge = EdgeSizes {
            top: t,
            right: r,
            bottom: b,
            left: l,
        };
    }
}

pub(crate) fn apply_border_side_width(side: &mut f32, values: &[Value], env: &LengthEnv<'_>) {
    if let [v] = values {
        if let Some(w) = border_width_value(v, env) {
            *side = w;
        }
    }
}

/// `border-style: <line-style>{1,4}` and `border-<side>-style` (mask = that side, one value).
pub(crate) fn apply_border_style(state: &mut BorderState, mask: u8, values: &[Value]) {
    let values = &*substitute_var_fallbacks(values);
    if mask != ALL_SIDES {
        if let [v] = values {
            if let Some(visible) = border_style_value(v) {
                set_styled(state, mask, visible);
            }
        }
        return;
    }
    let parsed: Option<Vec<bool>> = values.iter().map(border_style_value).collect();
    if let Some((t, r, b, l)) = parsed.as_deref().and_then(expand_four_sides) {
        set_styled(state, SIDE_TOP, t);
        set_styled(state, SIDE_RIGHT, r);
        set_styled(state, SIDE_BOTTOM, b);
        set_styled(state, SIDE_LEFT, l);
    }
}

/// `border-color: <color>{1,4}`. Only one color is painted, so the first one wins.
pub(crate) fn apply_border_color(
    style: &mut ComputedStyle,
    state: &mut BorderState,
    values: &[Value],
) {
    let values = &*substitute_var_fallbacks(values);
    let parsed: Option<Vec<Option<Color>>> = values.iter().map(border_color_value).collect();
    if let Some((t, _, _, _)) = parsed.as_deref().and_then(expand_four_sides) {
        match t {
            Some(c) => {
                style.border_color = c;
                state.explicit_color = true;
            }
            None => state.explicit_color = false,
        }
    }
}

fn background_color_of(val: &Value) -> Option<Option<Color>> {
    match val {
        Value::Color(c) => Some(if c.a == 0 { None } else { Some(*c) }),
        Value::Keyword(kw) if kw == "transparent" => Some(None),
        _ => None,
    }
}

pub(crate) fn apply_background_color(target: &mut Option<Color>, values: &[Value]) {
    if let [v] = values {
        if let Some(c) = background_color_of(v) {
            *target = c;
        }
    }
}

/// `background` shorthand. Only the color layer is rendered; every other component (images,
/// position, repeat, ...) is accepted and ignored, and a missing color resets to transparent.
/// A gradient with no separate color falls back to its first color stop.
pub(crate) fn apply_background(target: &mut Option<Color>, values: &[Value]) {
    let values = &*substitute_var_fallbacks(values);
    let mut color: Option<Option<Color>> = None;
    let mut gradient_color: Option<Color> = None;
    for val in values {
        if let Some(c) = background_color_of(val) {
            if color.replace(c).is_some() {
                return;
            }
            continue;
        }
        if let Value::Keyword(kw) = val {
            if matches!(kw.as_str(), "inherit" | "initial" | "unset" | "revert") {
                if kw != "inherit" && values.len() == 1 {
                    *target = None;
                }
                return;
            }
            if gradient_color.is_none() && kw.contains("gradient(") {
                gradient_color = first_color_in_function(kw);
            }
        }
    }
    *target = match color {
        Some(c) => c,
        None => gradient_color,
    };
}

fn first_color_in_function(func: &str) -> Option<Color> {
    let open = func.find('(')?;
    let inner = func[open + 1..]
        .strip_suffix(')')
        .unwrap_or(&func[open + 1..]);
    css::parse_values(inner).into_iter().find_map(|v| match v {
        Value::Color(c) if c.a > 0 => Some(c),
        _ => None,
    })
}

/// `box-sizing: content-box | border-box | inherit`.
pub(crate) fn apply_box_sizing(style: &mut ComputedStyle, values: &[Value], parent: BoxSizing) {
    if let [Value::Keyword(kw)] = values {
        match kw.as_str() {
            "content-box" | "initial" => style.box_sizing = BoxSizing::ContentBox,
            "border-box" => style.box_sizing = BoxSizing::BorderBox,
            "inherit" => style.box_sizing = parent,
            _ => {}
        }
    }
}

fn non_neg_length_pct(val: &Value, env: &LengthEnv<'_>) -> Option<LengthPct> {
    match val {
        Value::Length(pct, Unit::Percent) if *pct >= 0.0 => Some(LengthPct::Percent(*pct)),
        _ => resolve_length(val, env)
            .filter(|n| *n >= 0.0)
            .map(LengthPct::Px),
    }
}

/// `min-width` / `min-height`: `auto` | length | percentage (negative is invalid).
pub(crate) fn apply_min_size(target: &mut LengthPct, values: &[Value], env: &LengthEnv<'_>) {
    if let [v] = values {
        if matches!(v, Value::Keyword(k) if k == "auto" || k == "initial") {
            *target = LengthPct::Auto;
        } else if let Some(s) = non_neg_length_pct(v, env) {
            *target = s;
        }
    }
}

/// `width` / `height`: `auto` | length | percentage (negative is invalid).
pub(crate) fn apply_size(target: &mut LengthPct, values: &[Value], env: &LengthEnv<'_>) {
    apply_min_size(target, values, env);
}

/// `max-width` / `max-height`: `none` | length | percentage.
pub(crate) fn apply_max_size(target: &mut LengthPct, values: &[Value], env: &LengthEnv<'_>) {
    if let [v] = values {
        if matches!(v, Value::Keyword(k) if k == "none" || k == "initial") {
            *target = LengthPct::Auto;
        } else if let Some(s) = non_neg_length_pct(v, env) {
            *target = s;
        }
    }
}

fn inset_value(val: &Value, env: &LengthEnv<'_>) -> Option<LengthPct> {
    match val {
        Value::Keyword(k) if k == "auto" => Some(LengthPct::Auto),
        Value::Length(pct, Unit::Percent) => Some(LengthPct::Percent(*pct)),
        _ => resolve_length(val, env).map(LengthPct::Px),
    }
}

pub(crate) fn apply_inset_side(target: &mut LengthPct, values: &[Value], env: &LengthEnv<'_>) {
    if let [v] = values {
        if let Some(s) = inset_value(v, env) {
            *target = s;
        }
    }
}

/// `inset: <top> [<right> [<bottom> [<left>]]]`.
pub(crate) fn apply_inset(style: &mut ComputedStyle, values: &[Value], env: &LengthEnv<'_>) {
    let parsed: Option<Vec<LengthPct>> = values.iter().map(|v| inset_value(v, env)).collect();
    if let Some((t, r, b, l)) = parsed.as_deref().and_then(expand_four_sides) {
        style.inset.top = t;
        style.inset.right = r;
        style.inset.bottom = b;
        style.inset.left = l;
    }
}

/// The parts of a valid `font` shorthand this engine renders.
pub(crate) struct FontShorthand<'a> {
    pub size: &'a Value,
    pub bold: bool,
    pub monospace: bool,
}

fn is_font_size_value(val: &Value) -> bool {
    match val {
        Value::Length(n, _) => *n >= 0.0,
        Value::Keyword(kw) => matches!(
            kw.as_str(),
            "xx-small"
                | "x-small"
                | "small"
                | "medium"
                | "large"
                | "x-large"
                | "xx-large"
                | "xxx-large"
                | "smaller"
                | "larger"
        ),
        _ => false,
    }
}

/// Parses `font: [<style> || <variant> || <weight> || <stretch>]? <size> [/ <line-height>]?
/// <family>#`. Returns `None` for invalid values and for system fonts (`caption`, `menu`, ...),
/// which this engine does not model.
pub(crate) fn parse_font_shorthand(values: &[Value]) -> Option<FontShorthand<'_>> {
    let mut i = 0;
    let mut bold = false;
    let mut prefix = 0;
    while i < values.len() && prefix < 4 {
        let handled = match &values[i] {
            Value::Keyword(kw) => match kw.as_str() {
                "bold" | "bolder" => {
                    bold = true;
                    true
                }
                "lighter" => {
                    bold = false;
                    true
                }
                "normal" | "italic" | "oblique" | "small-caps" | "ultra-condensed"
                | "extra-condensed" | "condensed" | "semi-condensed" | "semi-expanded"
                | "expanded" | "extra-expanded" | "ultra-expanded" => true,
                _ => false,
            },
            Value::Number(n) if (1.0..=1000.0).contains(n) => {
                bold = *n >= 600.0;
                true
            }
            _ => false,
        };
        if !handled {
            break;
        }
        i += 1;
        prefix += 1;
    }
    let size = values.get(i).filter(|v| is_font_size_value(v))?;
    i += 1;
    if matches!(values.get(i), Some(Value::Keyword(k)) if k == "/") {
        match values.get(i + 1)? {
            Value::Number(n) | Value::Length(n, _) if *n >= 0.0 => {}
            Value::Keyword(k) if k == "normal" => {}
            _ => return None,
        }
        i += 2;
    }
    let families = &values[i..];
    if families.is_empty() {
        return None;
    }
    let mut monospace = false;
    for fam in families {
        let Value::Keyword(name) = fam else {
            return None;
        };
        if name == "/" || name.starts_with('!') {
            return None;
        }
        if is_monospace_font(&name.to_ascii_lowercase()) {
            monospace = true;
        }
    }
    Some(FontShorthand {
        size,
        bold,
        monospace,
    })
}
