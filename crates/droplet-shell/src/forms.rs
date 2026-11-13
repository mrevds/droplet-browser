//! Per-tab form focus, editing, selection, and submission state.
//!
//! Values live in the arena DOM (`value` for inputs, text for textarea/button,
//! `checked`/`selected` attributes), so only transient caret/selection/focus
//! state is stored here. Tab hibernation clears this state to prevent focus
//! leakage between pages.

use droplet_dom::{Document, NodeId, NodeKind};
use droplet_gfx::font;
use droplet_layout::{LayoutBox, BoxType};
use droplet_paint::{
    caret_chars, selection_chars, selection_spans, selection_text, ControlVisual, PageSelection,
    TextEntry, TextPosition,
};
use droplet_url::Url;

/// Maximum number of characters accepted into one control value.
pub const MAX_CONTROL_CHARS: usize = 16_384;

/// Caret movement requested by the platform layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaretMovement {
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
}

/// Transient focus, editing, and selection state owned by one tab.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FormState {
    pub focused: Option<NodeId>,
    /// Byte offset into the focused control's stored value.
    pub caret: usize,
    /// Byte offset where a control-text selection started, when selecting.
    pub anchor: Option<usize>,
    /// True while the mouse is dragging inside a focused text control.
    pub control_dragging: bool,
    /// Drag/select-all selection over rendered page text.
    pub page_selection: Option<PageSelection>,
    /// Fixed endpoint while dragging over page text.
    pub page_drag_anchor: Option<TextPosition>,
}

impl FormState {
    /// Clears all focus and selection state.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Ordered `(start, end)` byte range inside the focused control value.
    pub fn control_selection(&self) -> Option<(usize, usize)> {
        let anchor = self.anchor?;
        if anchor == self.caret {
            return None;
        }
        Some((anchor.min(self.caret), anchor.max(self.caret)))
    }

    /// Builds the paint-side visual snapshot for the current focus/selection.
    pub fn visual(&self, doc: &Document) -> droplet_paint::FormVisualState {
        let mut visual = droplet_paint::FormVisualState {
            focused: self.focused,
            page_selection: self.page_selection,
            ..Default::default()
        };
        let Some(node) = self.focused else {
            return visual;
        };
        if !is_text_like(doc, node) {
            return visual;
        }
        let value = value_of(doc, node);
        visual.control = ControlVisual {
            caret_chars: Some(caret_chars(&value, self.caret)),
            selection_chars: self
                .control_selection()
                .map(|(start, end)| selection_chars(&value, start, end)),
        };
        visual
    }
}

/// Returns the control kind of `node`, or `None` for non-controls.
pub fn control_kind(doc: &Document, node: NodeId) -> Option<droplet_layout::ControlKind> {
    droplet_layout::control_kind(doc, node)
}

/// True when the control accepts free-form text editing.
pub fn is_text_like(doc: &Document, node: NodeId) -> bool {
    control_kind(doc, node).is_some_and(|kind| kind.is_text_like())
}

/// True when the node is a form control that can receive focus.
pub fn is_focusable(doc: &Document, node: NodeId) -> bool {
    let Some(kind) = control_kind(doc, node) else {
        return false;
    };
    if kind == droplet_layout::ControlKind::Hidden {
        return false;
    }
    !doc
        .element(node)
        .is_some_and(|e| attr_truthy(e.attr("disabled")))
}

fn attr_truthy(raw: Option<&str>) -> bool {
    match raw {
        None => false,
        Some(value) => {
            let trimmed = value.trim();
            !(trimmed.eq_ignore_ascii_case("false") || trimmed == "0")
        }
    }
}

/// Focusable controls in DOM order (iterative traversal, hidden/disabled skipped).
pub fn focusable_controls(doc: &Document) -> Vec<NodeId> {
    let mut found = Vec::new();
    let mut stack = vec![doc.root()];
    while let Some(node) = stack.pop() {
        let Some(data) = doc.get(node) else {
            continue;
        };
        if let NodeKind::Element(elem) = &data.kind {
            let tag = elem.tag_name.as_str();
            if (tag.eq_ignore_ascii_case("input")
                || tag.eq_ignore_ascii_case("textarea")
                || tag.eq_ignore_ascii_case("select")
                || tag.eq_ignore_ascii_case("button"))
                && is_focusable(doc, node)
            {
                found.push(node);
            }
        }
        for &child in data.children.iter().rev() {
            stack.push(child);
        }
    }
    found
}

/// Next/previous focus target, wrapping at the ends.
pub fn next_focus(doc: &Document, current: Option<NodeId>, backwards: bool) -> Option<NodeId> {
    let controls = focusable_controls(doc);
    if controls.is_empty() {
        return None;
    }
    match current.and_then(|cur| controls.iter().position(|&n| n == cur)) {
        Some(idx) => {
            let next = if backwards {
                (idx + controls.len() - 1) % controls.len()
            } else {
                (idx + 1) % controls.len()
            };
            Some(controls[next])
        }
        None => Some(if backwards {
            *controls.last().expect("non-empty")
        } else {
            controls[0]
        }),
    }
}

/// Reads the current editable value of a control.
pub fn value_of(doc: &Document, node: NodeId) -> String {
    droplet_layout::control_value(doc, node)
}

/// Writes an edited value back into the arena DOM.
pub fn set_value(doc: &mut Document, node: NodeId, value: &str) {
    let Some(kind) = control_kind(doc, node) else {
        return;
    };
    match kind {
        droplet_layout::ControlKind::Textarea => doc.set_text(node, value),
        droplet_layout::ControlKind::TextInput | droplet_layout::ControlKind::PasswordInput => {
            doc.set_attr(node, "value", value);
        }
        _ => {}
    }
}

fn clamp_chars(value: String) -> String {
    if value.chars().count() <= MAX_CONTROL_CHARS {
        return value;
    }
    value.chars().take(MAX_CONTROL_CHARS).collect()
}

fn normalize_single_line(text: &str) -> String {
    text.chars()
        .filter(|ch| *ch != '\r' && *ch != '\n' && *ch != '\0')
        .collect()
}

/// Inserts `insert` at `caret`, replacing the current selection if present.
pub fn insert_text(
    doc: &mut Document,
    node: NodeId,
    insert: &str,
    caret: &mut usize,
    selection: Option<(usize, usize)>,
) -> bool {
    if insert.is_empty() {
        return false;
    }
    let kind = match control_kind(doc, node) {
        Some(kind) if kind.is_text_like() => kind,
        _ => return false,
    };
    let mut value = value_of(doc, node);
    let insert = if kind == droplet_layout::ControlKind::Textarea {
        insert.replace("\r\n", "\n").replace('\r', "\n")
    } else {
        normalize_single_line(insert)
    };
    let (start, end) = selection.unwrap_or((*caret, *caret));
    let start = floor_boundary(&value, start.min(value.len()));
    let end = ceil_boundary(&value, end.min(value.len()));
    value.replace_range(start..end, &insert);
    let value = clamp_chars(value);
    let new_caret = start + insert.len();
    *caret = new_caret.min(value.len());
    set_value(doc, node, &value);
    true
}

/// Deletes the character before the caret (or the selection).
pub fn delete_backward(
    doc: &mut Document,
    node: NodeId,
    caret: &mut usize,
    selection: Option<(usize, usize)>,
) -> bool {
    if let Some((start, end)) = selection {
        return delete_range(doc, node, start, end, caret);
    }
    let value = value_of(doc, node);
    if *caret == 0 {
        return false;
    }
    let prev = prev_boundary(&value, (*caret).min(value.len()));
    delete_range(doc, node, prev, *caret, caret)
}

/// Deletes the character after the caret (or the selection).
pub fn delete_forward(
    doc: &mut Document,
    node: NodeId,
    caret: &mut usize,
    selection: Option<(usize, usize)>,
) -> bool {
    if let Some((start, end)) = selection {
        return delete_range(doc, node, start, end, caret);
    }
    let value = value_of(doc, node);
    if *caret >= value.len() {
        return false;
    }
    let next = next_boundary(&value, *caret);
    delete_range(doc, node, *caret, next, caret)
}

/// Removes `[start, end)` from a control value and places the caret at `start`.
pub fn delete_range(
    doc: &mut Document,
    node: NodeId,
    start: usize,
    end: usize,
    caret: &mut usize,
) -> bool {
    let mut value = value_of(doc, node);
    if start >= end || start >= value.len() {
        return false;
    }
    let start = floor_boundary(&value, start);
    let end = ceil_boundary(&value, end.min(value.len()));
    if end <= start {
        return false;
    }
    value.replace_range(start..end, "");
    *caret = start.min(value.len());
    set_value(doc, node, &value);
    true
}

/// Moves a byte caret offset using UTF-8-safe boundaries.
pub fn move_caret(text: &str, caret: usize, movement: CaretMovement) -> usize {
    let caret = caret.min(text.len());
    match movement {
        CaretMovement::Left => prev_boundary(text, caret),
        CaretMovement::Right => next_boundary(text, caret),
        CaretMovement::Home => line_start(text, caret),
        CaretMovement::End => line_end(text, caret),
        CaretMovement::Up => vertical_caret(text, caret, -1),
        CaretMovement::Down => vertical_caret(text, caret, 1),
    }
}

fn prev_boundary(text: &str, mut idx: usize) -> usize {
    idx = idx.min(text.len());
    while idx > 0 && !text.is_char_boundary(idx) {
        idx -= 1;
    }
    if idx == 0 {
        return 0;
    }
    let mut prev = idx - 1;
    while prev > 0 && !text.is_char_boundary(prev) {
        prev -= 1;
    }
    prev
}

fn next_boundary(text: &str, mut idx: usize) -> usize {
    idx = idx.min(text.len());
    while idx < text.len() && !text.is_char_boundary(idx) {
        idx += 1;
    }
    if idx >= text.len() {
        return text.len();
    }
    let mut next = idx + 1;
    while next < text.len() && !text.is_char_boundary(next) {
        next += 1;
    }
    next
}

fn floor_boundary(text: &str, mut idx: usize) -> usize {
    idx = idx.min(text.len());
    while idx > 0 && !text.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

fn ceil_boundary(text: &str, mut idx: usize) -> usize {
    idx = idx.min(text.len());
    while idx < text.len() && !text.is_char_boundary(idx) {
        idx += 1;
    }
    idx
}

fn line_start(text: &str, caret: usize) -> usize {
    text[..floor_boundary(text, caret)].rfind('\n').map_or(0, |i| i + 1)
}

fn line_end(text: &str, caret: usize) -> usize {
    let start = floor_boundary(text, caret);
    text[start..]
        .find('\n')
        .map_or(text.len(), |i| start + i)
}

fn line_char_col(text: &str, caret: usize) -> (usize, usize) {
    let caret = floor_boundary(text, caret);
    let start = line_start(text, caret);
    (start, text[start..caret].chars().count())
}

fn vertical_caret(text: &str, caret: usize, delta: isize) -> usize {
    let caret = floor_boundary(text, caret);
    let (_, col) = line_char_col(text, caret);
    let line_idx = text[..line_start(text, caret)].matches('\n').count() as isize;
    let target = line_idx + delta;
    if target < 0 {
        return 0;
    }
    let lines: Vec<&str> = text.split('\n').collect();
    let target = target as usize;
    if target >= lines.len() {
        return text.len();
    }
    let start: usize = lines
        .iter()
        .take(target)
        .map(|line| line.len() + 1)
        .sum();
    start + byte_at_col(lines[target], col)
}

fn byte_at_col(line: &str, col: usize) -> usize {
    line.char_indices()
        .nth(col)
        .map(|(i, _)| i)
        .unwrap_or(line.len())
}

/// Ordered selection range from arbitrary anchor/caret byte offsets.
pub fn selection_range(anchor: usize, caret: usize) -> Option<(usize, usize)> {
    if anchor == caret {
        None
    } else {
        Some((anchor.min(caret), anchor.max(caret)))
    }
}

/// Returns the selected slice for a control value, clamped to UTF-8 boundaries.
pub fn selected_slice(text: &str, anchor: usize, caret: usize) -> Option<&str> {
    let (start, end) = selection_range(anchor, caret)?;
    let start = floor_boundary(text, start);
    let end = ceil_boundary(text, end.min(text.len()));
    (end > start).then(|| &text[start..end])
}

/// Toggles a checkbox (radios stay checked once selected).
pub fn toggle_checked(doc: &mut Document, node: NodeId) -> bool {
    let Some(kind) = control_kind(doc, node) else {
        return false;
    };
    if kind == droplet_layout::ControlKind::Checkbox {
        let checked = doc
            .element(node)
            .is_some_and(|e| attr_truthy(e.attr("checked")));
        if checked {
            doc.remove_attr(node, "checked");
        } else {
            doc.set_attr(node, "checked", "checked");
        }
        return true;
    }
    if kind == droplet_layout::ControlKind::Radio {
        check_radio(doc, node);
        return true;
    }
    false
}

/// Selects a radio and clears siblings with the same name in the same form/group.
pub fn check_radio(doc: &mut Document, node: NodeId) -> bool {
    let Some(display) = doc.element(node) else {
        return false;
    };
    let name = display.attr("name").unwrap_or("").to_string();
    if name.is_empty() {
        doc.set_attr(node, "checked", "checked");
        return true;
    }
    let scope = form_ancestor(doc, node);
    let controls = focusable_controls(doc);
    for other in controls {
        if other == node {
            continue;
        }
        if control_kind(doc, other) != Some(droplet_layout::ControlKind::Radio) {
            continue;
        }
        let same_group = doc
            .element(other)
            .is_some_and(|e| e.attr("name").is_some_and(|n| n == name));
        if same_group && form_ancestor(doc, other) == scope {
            doc.remove_attr(other, "checked");
        }
    }
    doc.set_attr(node, "checked", "checked");
    true
}

/// Cycles the selected index of a `<select>` by `delta`, wrapping at the ends.
pub fn cycle_select(doc: &mut Document, node: NodeId, delta: isize) -> bool {
    if control_kind(doc, node) != Some(droplet_layout::ControlKind::Select) {
        return false;
    }
    let options: Vec<NodeId> = doc
        .get(node)
        .map(|n| {
            n.children
                .iter()
                .copied()
                .filter(|&child| {
                    doc.element(child)
                        .is_some_and(|e| e.tag_name.eq_ignore_ascii_case("option"))
                })
                .collect()
        })
        .unwrap_or_default();
    if options.is_empty() {
        return false;
    }
    let current = droplet_layout::selected_option(doc, node)
        .and_then(|selected| options.iter().position(|&o| o == selected))
        .unwrap_or(0);
    let next = (current as isize + delta).rem_euclid(options.len() as isize) as usize;
    for option in &options {
        doc.remove_attr(*option, "selected");
    }
    doc.set_attr(options[next], "selected", "selected");
    true
}

/// Immediate `<form>` ancestor of `node`, if any.
pub fn form_ancestor(doc: &Document, node: NodeId) -> Option<NodeId> {
    let mut current = doc.get(node)?.parent;
    while let Some(parent) = current {
        if doc
            .element(parent)
            .is_some_and(|e| e.tag_name.eq_ignore_ascii_case("form"))
        {
            return Some(parent);
        }
        current = doc.get(parent)?.parent;
    }
    None
}

/// One successful form control (`name=value`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormPair {
    pub name: String,
    pub value: String,
}

/// Collects successful controls for `form`, in document order.
pub fn collect_form_pairs(
    doc: &Document,
    form: NodeId,
    submitter: Option<NodeId>,
) -> Vec<FormPair> {
    let mut pairs = Vec::new();
    let mut stack = vec![form];
    while let Some(node) = stack.pop() {
        let Some(data) = doc.get(node) else {
            continue;
        };
        if let NodeKind::Element(elem) = &data.kind {
            if attr_truthy(elem.attr("disabled")) {
                continue;
            }
            let tag = elem.tag_name.as_str();
            let name = elem.attr("name").unwrap_or("");
            if !name.is_empty() {
                if tag.eq_ignore_ascii_case("input") {
                    let ty = elem.attr("type").unwrap_or("text");
                    let kind = control_kind(doc, node);
                    let include = match kind {
                        Some(droplet_layout::ControlKind::Checkbox)
                        | Some(droplet_layout::ControlKind::Radio) => {
                            attr_truthy(elem.attr("checked"))
                        }
                        Some(droplet_layout::ControlKind::Submit) => submitter == Some(node),
                        Some(droplet_layout::ControlKind::Hidden) => true,
                        Some(droplet_layout::ControlKind::TextInput)
                        | Some(droplet_layout::ControlKind::PasswordInput) => true,
                        _ => ty.eq_ignore_ascii_case("text") || ty.is_empty(),
                    };
                    if include {
                        pairs.push(FormPair {
                            name: name.to_string(),
                            value: droplet_layout::control_value(doc, node),
                        });
                    }
                } else if tag.eq_ignore_ascii_case("textarea") {
                    pairs.push(FormPair {
                        name: name.to_string(),
                        value: doc.text_content(node),
                    });
                } else if tag.eq_ignore_ascii_case("select") {
                    if let Some(option) = droplet_layout::selected_option(doc, node) {
                        let option_disabled = doc
                            .element(option)
                            .is_some_and(|e| attr_truthy(e.attr("disabled")));
                        if !option_disabled {
                            let value = doc
                                .element(option)
                                .and_then(|e| e.attr("value").map(str::to_string))
                                .unwrap_or_else(|| doc.text_content(option).trim().to_string());
                            pairs.push(FormPair {
                                name: name.to_string(),
                                value,
                            });
                        }
                    }
                } else if tag.eq_ignore_ascii_case("button") {
                    let ty = elem.attr("type").unwrap_or("submit");
                    if submitter == Some(node) && ty.eq_ignore_ascii_case("submit") {
                        pairs.push(FormPair {
                            name: name.to_string(),
                            value: elem.attr("value").unwrap_or("").to_string(),
                        });
                    }
                }
            }
            if tag.eq_ignore_ascii_case("option") {
                continue;
            }
        }
        for &child in data.children.iter().rev() {
            stack.push(child);
        }
    }
    pairs
}

/// HTTP method used by a form submission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormMethod {
    Get,
    Post,
}

impl FormMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            FormMethod::Get => "get",
            FormMethod::Post => "post",
        }
    }
}

/// Prepared navigation request for a form submission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormSubmission {
    pub url: Url,
    pub method: FormMethod,
    pub body: String,
    pub pairs: Vec<FormPair>,
}

/// Resolves a form action against `base_url` and builds the GET query or POST body.
pub fn prepare_submission(
    doc: &Document,
    trigger: NodeId,
    base_url: &Url,
) -> Option<FormSubmission> {
    let form = form_ancestor(doc, trigger)?;
    let elem = doc.element(form)?;
    let action = elem.attr("action").unwrap_or("").trim();
    let method = match elem.attr("method").unwrap_or("get").trim() {
        m if m.eq_ignore_ascii_case("post") => FormMethod::Post,
        _ => FormMethod::Get,
    };
    let url = if action.is_empty() {
        base_url.clone()
    } else {
        base_url.resolve(action).ok()?
    };
    let submitter = if control_kind(doc, trigger)
        .is_some_and(|k| k.is_button_like())
    {
        Some(trigger)
    } else {
        None
    };
    let pairs = collect_form_pairs(doc, form, submitter);
    let body = encode_pairs(&pairs);
    let url = match method {
        FormMethod::Get => url_with_query(&url, &body)?,
        FormMethod::Post => url,
    };
    Some(FormSubmission {
        url,
        method,
        body,
        pairs,
    })
}

fn url_with_query(base: &Url, query: &str) -> Option<Url> {
    let mut raw = base.as_str().split('#').next().unwrap_or("").to_string();
    if let Some(idx) = raw.find('?') {
        raw.truncate(idx);
    }
    if !query.is_empty() {
        raw.push('?');
        raw.push_str(query);
    }
    Url::parse(&raw).ok()
}

/// RFC 3986 percent-encodes one form component (space becomes `%20`).
pub fn percent_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        let unreserved = byte.is_ascii_alphanumeric()
            || matches!(byte, b'-' | b'_' | b'.' | b'~');
        if unreserved {
            out.push(byte as char);
        } else {
            out.push('%');
            out.push(hex_digit(byte >> 4));
            out.push(hex_digit(byte & 0x0F));
        }
    }
    out
}

fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => (b'0' + value) as char,
        _ => (b'A' + value - 10) as char,
    }
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Decodes `%XX` escapes and `+` as space; invalid escapes are kept verbatim.
pub fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut idx = 0usize;
    while idx < bytes.len() {
        match bytes[idx] {
            b'+' => {
                out.push(b' ');
                idx += 1;
            }
            b'%' if idx + 2 < bytes.len() => {
                match (hex_value(bytes[idx + 1]), hex_value(bytes[idx + 2])) {
                    (Some(hi), Some(lo)) => {
                        out.push((hi << 4) | lo);
                        idx += 3;
                    }
                    _ => {
                        out.push(b'%');
                        idx += 1;
                    }
                }
            }
            byte => {
                out.push(byte);
                idx += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Encodes pairs as `a=1&b=%D0%9F`.
pub fn encode_pairs(pairs: &[FormPair]) -> String {
    let mut out = String::new();
    for (idx, pair) in pairs.iter().enumerate() {
        if idx > 0 {
            out.push('&');
        }
        out.push_str(&percent_encode(&pair.name));
        out.push('=');
        out.push_str(&percent_encode(&pair.value));
    }
    out
}

/// Decodes an `a=1&b=2` query/body back into pairs.
pub fn decode_pairs(input: &str) -> Vec<FormPair> {
    let mut out = Vec::new();
    for part in input.split('&') {
        if part.is_empty() {
            continue;
        }
        let (name, value) = part.split_once('=').unwrap_or((part, ""));
        out.push(FormPair {
            name: percent_decode(name),
            value: percent_decode(value),
        });
    }
    out
}

/// Finds the rendered text box under `(x, y)` and converts the x position into a
/// byte offset inside that box's concatenated lines.
pub fn text_position_at(root: &LayoutBox, x: f32, y: f32) -> Option<TextPosition> {
    let mut stack = vec![root];
    let mut hit: Option<&LayoutBox> = None;
    while let Some(node) = stack.pop() {
        if matches!(node.box_type, BoxType::Text { .. }) {
            let rect = node.dimensions.content;
            if x >= rect.x
                && x <= rect.x + rect.width
                && y >= rect.y
                && y <= rect.y + rect.height
            {
                hit = Some(node);
            }
        }
        for child in node.children.iter().rev() {
            stack.push(child);
        }
    }
    let node = hit?;
    let BoxType::Text {
        lines,
        line_height,
        font_size,
        monospace,
        ..
    } = &node.box_type
    else {
        return None;
    };
    let rect = node.dimensions.content;
    let mut line_idx = if *line_height > 0.0 {
        ((y - rect.y) / *line_height).floor() as usize
    } else {
        0
    };
    line_idx = line_idx.min(lines.len().saturating_sub(1));
    let line = lines.get(line_idx)?;
    let mut byte_in_line = 0usize;
    if x > rect.x {
        for (idx, ch) in line.char_indices() {
            let end = idx + ch.len_utf8();
            let w = font::measure_text_px(&line[..end], *font_size, node.style.bold, *monospace);
            if rect.x + w <= x {
                byte_in_line = end;
            } else {
                let prev_w = font::measure_text_px(
                    &line[..idx],
                    *font_size,
                    node.style.bold,
                    *monospace,
                );
                if x - (rect.x + prev_w) > (w - prev_w) * 0.5 {
                    byte_in_line = end;
                }
                break;
            }
        }
        if x >= rect.x + font::measure_text_px(line, *font_size, node.style.bold, *monospace) {
            byte_in_line = line.len();
        }
    }
    let offset = lines.iter().take(line_idx).map(String::len).sum::<usize>() + byte_in_line;
    Some(TextPosition {
        node: node.node_id?,
        offset,
    })
}

/// Finds the layout box for a control node.
pub fn find_control_box(root: &LayoutBox, node: NodeId) -> Option<&LayoutBox> {
    let mut stack = vec![root];
    while let Some(current) = stack.pop() {
        if current.node_id == Some(node) && matches!(current.box_type, BoxType::Control(_)) {
            return Some(current);
        }
        for child in current.children.iter().rev() {
            stack.push(child);
        }
    }
    None
}

/// Converts a pointer position inside a control to a character column in its
/// visible text. Used for click-to-caret and drag selection.
pub fn control_char_at(root: &LayoutBox, node: NodeId, x: f32, y: f32) -> Option<usize> {
    let layout_box = find_control_box(root, node)?;
    let BoxType::Control(control) = &layout_box.box_type else {
        return None;
    };
    if !control.kind.is_text_like() {
        return None;
    }
    let rect = layout_box.dimensions.content;
    let style = layout_box.style;
    let line_height = font::line_height_px(style.font_size).max(1.0);
    let text = if control.kind == droplet_layout::ControlKind::Textarea {
        control.text.as_str()
    } else {
        control.text.split('\n').next().unwrap_or("")
    };
    let line_idx = if control.kind == droplet_layout::ControlKind::Textarea {
        let max_line = control.text.split('\n').count().saturating_sub(1);
        let rel_y = (y - rect.y).max(0.0);
        ((rel_y / line_height).floor() as usize).min(max_line)
    } else {
        0
    };
    let line = if control.kind == droplet_layout::ControlKind::Textarea {
        control.text.split('\n').nth(line_idx).unwrap_or("")
    } else {
        text
    };
    let mut char_col = 0usize;
    if x > rect.x {
        for (idx, ch) in line.char_indices() {
            let end = idx + ch.len_utf8();
            let w =
                font::measure_text_px(&line[..end], style.font_size, style.bold, style.monospace);
            if rect.x + w <= x {
                char_col += 1;
            } else {
                let prev_w = font::measure_text_px(
                    &line[..idx],
                    style.font_size,
                    style.bold,
                    style.monospace,
                );
                if x - (rect.x + prev_w) > (w - prev_w) * 0.5 {
                    char_col += 1;
                }
                break;
            }
        }
        if x >= rect.x + font::measure_text_px(line, style.font_size, style.bold, style.monospace)
        {
            char_col = line.chars().count();
        }
    }
    if control.kind == droplet_layout::ControlKind::Textarea {
        let line_start_chars: usize = control
            .text
            .split('\n')
            .take(line_idx)
            .map(|line| line.chars().count() + 1)
            .sum();
        Some(line_start_chars + char_col)
    } else {
        Some(char_col)
    }
}

/// Converts a character index into a byte offset inside `text`.
pub fn byte_at_char(text: &str, chars: usize) -> usize {
    text.char_indices()
        .nth(chars)
        .map(|(idx, _)| idx)
        .unwrap_or(text.len())
}

/// Ordered rendered-text entries used for page selection and clipboard copy.
pub fn text_entries(root: &LayoutBox) -> Vec<TextEntry> {
    droplet_paint::forms_overlay::collect_text_entries(root)
}

/// Copies the current page selection (or returns an empty string).
pub fn page_selection_text(root: &LayoutBox, selection: &PageSelection) -> String {
    selection_text(&text_entries(root), selection)
}

/// Returns the rectangles representing a control selection, for tests/tools.
pub fn control_selection_spans(
    value: &str,
    selection: Option<(usize, usize)>,
) -> Vec<(usize, usize)> {
    let Some((start, end)) = selection else {
        return Vec::new();
    };
    let start = floor_boundary(value, start);
    let end = ceil_boundary(value, end.min(value.len()));
    if end > start {
        vec![(start, end)]
    } else {
        Vec::new()
    }
}

/// Keeps only displayable text and rejects NUL bytes from OS clipboards.
pub fn sanitize_clipboard_text(text: &str) -> String {
    text.replace('\0', "").replace("\r\n", "\n").replace('\r', "\n")
}

/// Clipboard text normalized for a single-line input.
pub fn single_line_clipboard_text(text: &str) -> String {
    sanitize_clipboard_text(text)
        .chars()
        .map(|ch| if ch == '\n' { ' ' } else { ch })
        .collect()
}

/// Returns the page selection covering all rendered text.
pub fn select_all_page(root: &LayoutBox) -> Option<PageSelection> {
    let entries = text_entries(root);
    let last = entries.last()?;
    Some(PageSelection {
        anchor: TextPosition {
            node: entries[0].node,
            offset: 0,
        },
        focus: TextPosition {
            node: last.node,
            offset: last.text.len(),
        },
    })
}

/// Number of selected entries (mostly useful in tests and diagnostics).
pub fn selection_entry_count(entries: &[TextEntry], selection: &PageSelection) -> usize {
    selection_spans(entries, selection).len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use droplet_dom::html;
    use droplet_layout::ControlKind;

    #[test]
    fn control_parsing_and_focus_traversal_skip_disabled_and_hidden() {
        let doc = html::parse(
            r#"<form action="/go" method="POST">
                <input type="hidden" name="csrf" value="abc">
                <input type="text" name="u" placeholder="Имя">
                <input type="password" name="p" disabled>
                <textarea name="bio">Привет</textarea>
                <select name="role"><option>One</option><option selected>Two</option></select>
                <button type="submit">Send</button>
            </form>"#,
        );
        let controls = focusable_controls(&doc);
        assert_eq!(controls.len(), 4);
        assert_eq!(control_kind(&doc, controls[0]), Some(ControlKind::TextInput));
        assert_eq!(control_kind(&doc, controls[1]), Some(ControlKind::Textarea));
        assert_eq!(control_kind(&doc, controls[2]), Some(ControlKind::Select));
        assert_eq!(control_kind(&doc, controls[3]), Some(ControlKind::Button));

        // Tab forward + wrap
        assert_eq!(next_focus(&doc, None, false), Some(controls[0]));
        assert_eq!(next_focus(&doc, Some(controls[0]), false), Some(controls[1]));
        assert_eq!(next_focus(&doc, Some(controls[3]), false), Some(controls[0]));

        // Shift+Tab backward + wrap
        assert_eq!(next_focus(&doc, None, true), Some(controls[3]));
        assert_eq!(next_focus(&doc, Some(controls[0]), true), Some(controls[3]));
        assert_eq!(next_focus(&doc, Some(controls[2]), true), Some(controls[1]));
    }

    #[test]
    fn utf8_cyrillic_editing_caret_and_selection_ranges() {
        let mut doc = html::parse(r#"<input type="text" value="Мир">"#);
        let input = focusable_controls(&doc)[0];
        let mut caret = 0;

        assert!(insert_text(&mut doc, input, "Привет, ", &mut caret, None));
        assert_eq!(value_of(&doc, input), "Привет, Мир");
        assert_eq!(caret, "Привет, ".len());

        // Move left across space so caret sits between ',' and ' '
        caret = move_caret(&value_of(&doc, input), caret, CaretMovement::Left);
        assert!(value_of(&doc, input).is_char_boundary(caret));

        // Backspace & delete forward
        assert!(delete_backward(&mut doc, input, &mut caret, None));
        assert_eq!(value_of(&doc, input), "Привет Мир");
        assert!(delete_forward(&mut doc, input, &mut caret, None));
        assert_eq!(value_of(&doc, input), "ПриветМир");

        // Selection slice clamping on mid-codepoint byte offsets never panics
        let val = value_of(&doc, input);
        let slice = selected_slice(&val, 1, 5).expect("valid slice");
        assert!(slice.is_char_boundary(0));
        assert!(slice.is_char_boundary(slice.len()));

        // Replace selection with new text
        assert!(insert_text(&mut doc, input, "!", &mut caret, Some((0, val.len()))));
        assert_eq!(value_of(&doc, input), "!");

        // Multiline textarea Up/Down/Home/End
        let area_doc = html::parse("<textarea>Первая\nВторая строка</textarea>");
        let area = focusable_controls(&area_doc)[0];
        let area_val = value_of(&area_doc, area);
        let end_of_first = move_caret(&area_val, 0, CaretMovement::End);
        assert_eq!(&area_val[..end_of_first], "Первая");
        let down = move_caret(&area_val, end_of_first, CaretMovement::Down);
        assert!(down > end_of_first);
        let home = move_caret(&area_val, down, CaretMovement::Home);
        assert_eq!(&area_val[home..], "Вторая строка");
    }

    #[test]
    fn checkbox_radio_groups_and_select_cycling() {
        let mut doc = html::parse(
            r#"<form id="f1">
                <input type="checkbox" name="agree">
                <input type="radio" name="plan" value="free" checked>
                <input type="radio" name="plan" value="pro">
                <select name="lang"><option value="ru">RU</option><option value="en">EN</option></select>
            </form>
            <form id="f2">
                <input type="radio" name="plan" value="other" checked>
            </form>"#,
        );
        let ctrls = focusable_controls(&doc);
        let (chk, r1, r2, sel, r_other) = (ctrls[0], ctrls[1], ctrls[2], ctrls[3], ctrls[4]);

        assert!(toggle_checked(&mut doc, chk));
        assert!(doc.element(chk).unwrap().attr("checked").is_some());
        assert!(toggle_checked(&mut doc, chk));
        assert!(doc.element(chk).unwrap().attr("checked").is_none());

        // Clicking r2 unchecks r1 in f1, leaves r_other in f2 untouched
        assert!(toggle_checked(&mut doc, r2));
        assert!(doc.element(r1).unwrap().attr("checked").is_none());
        assert!(doc.element(r2).unwrap().attr("checked").is_some());
        assert!(doc.element(r_other).unwrap().attr("checked").is_some());

        // Select cycles forward & backward with wrap
        assert!(cycle_select(&mut doc, sel, 1));
        assert_eq!(
            doc.text_content(droplet_layout::selected_option(&doc, sel).unwrap()),
            "EN"
        );
        assert!(cycle_select(&mut doc, sel, 1));
        assert_eq!(
            doc.text_content(droplet_layout::selected_option(&doc, sel).unwrap()),
            "RU"
        );
        assert!(cycle_select(&mut doc, sel, -1));
        assert_eq!(
            doc.text_content(droplet_layout::selected_option(&doc, sel).unwrap()),
            "EN"
        );
    }

    #[test]
    fn url_encoding_and_form_submission_get_and_post() {
        let pairs = vec![
            FormPair {
                name: "q".into(),
                value: "Привет мир!".into(),
            },
            FormPair {
                name: "a&b".into(),
                value: "1=2".into(),
            },
        ];
        let encoded = encode_pairs(&pairs);
        assert_eq!(
            encoded,
            "q=%D0%9F%D1%80%D0%B8%D0%B2%D0%B5%D1%82%20%D0%BC%D0%B8%D1%80%21&a%26b=1%3D2"
        );
        assert_eq!(decode_pairs(&encoded), pairs);
        assert_eq!(percent_decode("hello+world%ZZ"), "hello world%ZZ");

        let doc = html::parse(
            r#"<form action="/search" method="get">
                <input type="text" name="q" value="тест">
                <input type="checkbox" name="opt" value="yes">
                <input type="checkbox" name="on" value="1" checked>
                <input type="text" name="ignored" value="x" disabled>
                <input type="submit" name="btn" value="Go">
            </form>"#,
        );
        let ctrls = focusable_controls(&doc);
        let base = Url::parse("http://127.0.0.1:8080/page?old=1").unwrap();
        let sub = prepare_submission(&doc, ctrls[3], &base).unwrap();
        assert_eq!(sub.method, FormMethod::Get);
        assert_eq!(
            sub.url.as_str(),
            "http://127.0.0.1:8080/search?q=%D1%82%D0%B5%D1%81%D1%82&on=1&btn=Go"
        );
    }
}
