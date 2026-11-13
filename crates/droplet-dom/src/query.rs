use crate::arena::{Document, Node, NodeId, NodeKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Combinator {
    Descendant,
    Child,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AttrFilter {
    name: String,
    value: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct CompoundSelector {
    tag: Option<String>,
    id: Option<String>,
    classes: Vec<String>,
    attrs: Vec<AttrFilter>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ComplexSelector {
    /// Sequence from left to right; combinator[i] connects part[i] and part[i + 1].
    parts: Vec<CompoundSelector>,
    combinators: Vec<Combinator>,
}

pub(crate) fn find_element_by_id(doc: &Document, id: &str) -> Option<NodeId> {
    if id.is_empty() {
        return None;
    }
    let mut stack = vec![doc.root()];
    while let Some(curr) = stack.pop() {
        let Some(node) = doc.get(curr) else {
            continue;
        };
        if let NodeKind::Element(data) = &node.kind {
            if data.id() == Some(id) {
                return Some(curr);
            }
        }
        for &child in node.children.iter().rev() {
            stack.push(child);
        }
    }
    None
}

pub(crate) fn query_first(doc: &Document, selector: &str) -> Option<NodeId> {
    let selectors = parse_selector_list(selector)?;
    let mut stack = vec![doc.root()];
    while let Some(curr) = stack.pop() {
        let Some(node) = doc.get(curr) else {
            continue;
        };
        if matches!(node.kind, NodeKind::Element(_))
            && selectors.iter().any(|sel| matches_complex(doc, curr, sel))
        {
            return Some(curr);
        }
        for &child in node.children.iter().rev() {
            stack.push(child);
        }
    }
    None
}

pub(crate) fn query_all(doc: &Document, selector: &str) -> Vec<NodeId> {
    let Some(selectors) = parse_selector_list(selector) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut stack = vec![doc.root()];
    while let Some(curr) = stack.pop() {
        let Some(node) = doc.get(curr) else {
            continue;
        };
        if matches!(node.kind, NodeKind::Element(_))
            && selectors.iter().any(|sel| matches_complex(doc, curr, sel))
        {
            out.push(curr);
        }
        for &child in node.children.iter().rev() {
            stack.push(child);
        }
    }
    out
}

/// Normalizes a JS inline style property (`backgroundColor`) or CSS property
/// (`background-color`) into lowercase CSS kebab-case.
pub(crate) fn normalize_style_prop_name(prop: &str) -> String {
    let trimmed = prop.trim();
    if trimmed.eq_ignore_ascii_case("cssFloat") {
        return "float".to_string();
    }
    if trimmed.starts_with("--") {
        return trimmed.to_string();
    }
    let mut out = String::with_capacity(trimmed.len() + 4);
    for (idx, ch) in trimmed.char_indices() {
        if ch.is_ascii_uppercase() {
            if idx > 0 && !out.ends_with('-') {
                out.push('-');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

pub(crate) fn style_prop_get(doc: &Document, id: NodeId, prop: &str) -> String {
    let Some(elem) = doc.element(id) else {
        return String::new();
    };
    let Some(raw) = elem.attr("style") else {
        return String::new();
    };
    let target = normalize_style_prop_name(prop);
    if target.is_empty() {
        return String::new();
    }
    let decls = parse_inline_style(raw);
    decls
        .into_iter()
        .rfind(|(k, _)| k.eq_ignore_ascii_case(&target))
        .map(|(_, v)| v)
        .unwrap_or_default()
}

pub(crate) fn style_prop_set(doc: &mut Document, id: NodeId, prop: &str, value: &str) {
    if doc.element(id).is_none() {
        return;
    }
    let target = normalize_style_prop_name(prop);
    if target.is_empty() {
        return;
    }

    let mut decls = doc
        .element(id)
        .and_then(|e| e.attr("style"))
        .map(parse_inline_style)
        .unwrap_or_default();

    let clean_val = value.trim();
    decls.retain(|(k, _)| !k.eq_ignore_ascii_case(&target));
    if !clean_val.is_empty() {
        decls.push((target, clean_val.to_string()));
    }

    if decls.is_empty() {
        doc.remove_attr(id, "style");
    } else {
        let mut serialized = String::new();
        for (idx, (k, v)) in decls.iter().enumerate() {
            if idx > 0 {
                serialized.push_str("; ");
            }
            serialized.push_str(k);
            serialized.push_str(": ");
            serialized.push_str(v);
        }
        doc.set_attr(id, "style", &serialized);
    }
}

fn parse_inline_style(raw: &str) -> Vec<(String, String)> {
    let mut decls = Vec::new();
    for item in split_declarations(raw) {
        let Some((name, val)) = item.split_once(':') else {
            continue;
        };
        let key = normalize_style_prop_name(name);
        let value = val.trim();
        if key.is_empty() || value.is_empty() {
            continue;
        }
        decls.push((key, value.to_string()));
    }
    decls
}

fn split_declarations(input: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut quote: Option<u8> = None;
    let mut paren_depth = 0usize;
    let bytes = input.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if let Some(q) = quote {
            if b == b'\\' {
                i = (i + 2).min(bytes.len());
                continue;
            }
            if b == q {
                quote = None;
            }
        } else {
            match b {
                b'"' | b'\'' => quote = Some(b),
                b'(' => paren_depth += 1,
                b')' => paren_depth = paren_depth.saturating_sub(1),
                b';' if paren_depth == 0 => {
                    parts.push(&input[start..i]);
                    start = i + 1;
                }
                _ => {}
            }
        }
        i += 1;
    }
    if start < input.len() {
        parts.push(&input[start..]);
    }
    parts
}

fn parse_selector_list(input: &str) -> Option<Vec<ComplexSelector>> {
    let items = split_top_level_commas(input);
    if items.is_empty() {
        return None;
    }
    let mut out = Vec::with_capacity(items.len());
    for raw in items {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return None;
        }
        let complex = parse_complex_selector(trimmed)?;
        out.push(complex);
    }
    Some(out)
}

fn split_top_level_commas(input: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut quote: Option<u8> = None;
    let mut bracket_depth = 0usize;
    let bytes = input.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if let Some(q) = quote {
            if b == b'\\' {
                i = (i + 2).min(bytes.len());
                continue;
            }
            if b == q {
                quote = None;
            }
        } else {
            match b {
                b'"' | b'\'' => quote = Some(b),
                b'[' => bracket_depth += 1,
                b']' => bracket_depth = bracket_depth.saturating_sub(1),
                b',' if bracket_depth == 0 => {
                    out.push(&input[start..i]);
                    start = i + 1;
                }
                _ => {}
            }
        }
        i += 1;
    }
    out.push(&input[start..]);
    out
}

fn parse_complex_selector(input: &str) -> Option<ComplexSelector> {
    let bytes = input.as_bytes();
    let mut pos = 0usize;
    let mut parts = Vec::new();
    let mut combinators = Vec::new();

    while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
        pos += 1;
    }
    while pos < bytes.len() {
        let (compound, next_pos) = parse_compound_selector(input, pos)?;
        parts.push(compound);
        pos = next_pos;

        let mut saw_ws = false;
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            saw_ws = true;
            pos += 1;
        }
        if pos >= bytes.len() {
            break;
        }
        if bytes[pos] == b'>' {
            combinators.push(Combinator::Child);
            pos += 1;
            while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
                pos += 1;
            }
            if pos >= bytes.len() {
                return None;
            }
        } else if saw_ws {
            combinators.push(Combinator::Descendant);
        } else {
            return None;
        }
    }

    if parts.is_empty() || parts.len() != combinators.len() + 1 {
        return None;
    }
    Some(ComplexSelector { parts, combinators })
}

fn parse_compound_selector(input: &str, mut pos: usize) -> Option<(CompoundSelector, usize)> {
    let bytes = input.as_bytes();
    let start = pos;
    let mut sel = CompoundSelector::default();

    if pos < bytes.len() && bytes[pos] == b'*' {
        pos += 1;
    } else if pos < bytes.len() && is_ident_start_byte(bytes[pos]) {
        let name_start = pos;
        pos = scan_ident(input, pos);
        sel.tag = Some(input[name_start..pos].to_ascii_lowercase());
    }

    while pos < bytes.len() {
        match bytes[pos] {
            b'#' => {
                pos += 1;
                let id_start = pos;
                pos = scan_ident(input, pos);
                if pos == id_start {
                    return None;
                }
                let id_val = &input[id_start..pos];
                if let Some(existing) = &sel.id {
                    if existing != id_val {
                        // Conflicting IDs cannot match; store sentinel? Keep impossible via attrs?
                        // Easier: reject or handle impossible match cleanly.
                        sel.id = Some("\0impossible".to_string());
                    }
                } else {
                    sel.id = Some(id_val.to_string());
                }
            }
            b'.' => {
                pos += 1;
                let cls_start = pos;
                pos = scan_ident(input, pos);
                if pos == cls_start {
                    return None;
                }
                sel.classes.push(input[cls_start..pos].to_string());
            }
            b'[' => {
                let (attr, next_pos) = parse_attr_filter(input, pos + 1)?;
                sel.attrs.push(attr);
                pos = next_pos;
            }
            b'>' => break,
            b if b.is_ascii_whitespace() => break,
            _ => return None,
        }
    }

    if pos == start {
        return None;
    }
    Some((sel, pos))
}

fn parse_attr_filter(input: &str, mut pos: usize) -> Option<(AttrFilter, usize)> {
    let bytes = input.as_bytes();
    while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
        pos += 1;
    }
    let name_start = pos;
    pos = scan_ident(input, pos);
    if pos == name_start {
        return None;
    }
    let name = input[name_start..pos].to_ascii_lowercase();
    while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
        pos += 1;
    }
    let mut value = None;
    if pos < bytes.len() && bytes[pos] == b'=' {
        pos += 1;
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if pos < bytes.len() && (bytes[pos] == b'"' || bytes[pos] == b'\'') {
            let q = bytes[pos];
            pos += 1;
            let val_start = pos;
            while pos < bytes.len() && bytes[pos] != q {
                if bytes[pos] == b'\\' && pos + 1 < bytes.len() {
                    pos += 2;
                } else {
                    pos += 1;
                }
            }
            if pos >= bytes.len() {
                return None;
            }
            value = Some(input[val_start..pos].to_string());
            pos += 1;
        } else {
            let val_start = pos;
            while pos < bytes.len() && !bytes[pos].is_ascii_whitespace() && bytes[pos] != b']' {
                pos += 1;
            }
            value = Some(input[val_start..pos].to_string());
        }
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
    }
    if pos >= bytes.len() || bytes[pos] != b']' {
        return None;
    }
    Some((AttrFilter { name, value }, pos + 1))
}

fn is_ident_start_byte(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b == b'-' || !b.is_ascii()
}

fn scan_ident(input: &str, mut pos: usize) -> usize {
    let bytes = input.as_bytes();
    while pos < bytes.len() {
        let b = bytes[pos];
        if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || !b.is_ascii() {
            pos += 1;
        } else {
            break;
        }
    }
    pos
}

fn matches_compound(doc: &Document, node_id: NodeId, sel: &CompoundSelector) -> bool {
    let Some(elem) = doc.element(node_id) else {
        return false;
    };
    if let Some(tag) = &sel.tag {
        if !elem.tag_name.eq_ignore_ascii_case(tag) {
            return false;
        }
    }
    if let Some(id) = &sel.id {
        if elem.id() != Some(id.as_str()) {
            return false;
        }
    }
    for cls in &sel.classes {
        if !elem.has_class(cls) {
            return false;
        }
    }
    for attr in &sel.attrs {
        match &attr.value {
            None => {
                if elem.attr(&attr.name).is_none() {
                    return false;
                }
            }
            Some(expected) => {
                if elem.attr(&attr.name) != Some(expected.as_str()) {
                    return false;
                }
            }
        }
    }
    true
}

fn matches_complex(doc: &Document, target_id: NodeId, sel: &ComplexSelector) -> bool {
    let last_idx = sel.parts.len() - 1;
    if !matches_compound(doc, target_id, &sel.parts[last_idx]) {
        return false;
    }
    if last_idx == 0 {
        return true;
    }

    // Iterative backtracking stack: (current_node_id, part_idx_matched_at_current_node)
    let mut stack = vec![(target_id, last_idx)];
    while let Some((curr_id, part_idx)) = stack.pop() {
        if part_idx == 0 {
            return true;
        }
        let comb = sel.combinators[part_idx - 1];
        let prev_part = &sel.parts[part_idx - 1];
        match comb {
            Combinator::Child => {
                if let Some(parent_id) = element_parent(doc, curr_id) {
                    if matches_compound(doc, parent_id, prev_part) {
                        stack.push((parent_id, part_idx - 1));
                    }
                }
            }
            Combinator::Descendant => {
                let mut anc = element_parent(doc, curr_id);
                let mut steps = 0usize;
                let max_steps = doc.nodes.len();
                let mut candidates = Vec::new();
                while let Some(anc_id) = anc {
                    steps += 1;
                    if steps > max_steps {
                        break;
                    }
                    if matches_compound(doc, anc_id, prev_part) {
                        candidates.push(anc_id);
                    }
                    anc = element_parent(doc, anc_id);
                }
                for anc_id in candidates.into_iter().rev() {
                    stack.push((anc_id, part_idx - 1));
                }
            }
        }
    }
    false
}

fn element_parent(doc: &Document, id: NodeId) -> Option<NodeId> {
    let parent_id = doc.get(id)?.parent?;
    match doc.get(parent_id) {
        Some(Node {
            kind: NodeKind::Element(_),
            ..
        }) => Some(parent_id),
        _ => None,
    }
}
