use std::cell::RefCell;
use std::sync::OnceLock;

use droplet_dom::{Document, ElementData, NodeId, NodeKind};

use crate::computed::{
    AlignItems, ComputedStyle, Display, FlexBasis, FlexDirection, FlexWrap, JustifyContent,
};
use crate::css::{
    self, Color, Combinator, ComplexSelector, CompoundSelector, Declaration, PseudoClass,
    Specificity, Stylesheet, Unit, Value,
};
use crate::rule_index::{AncestorBloom, RuleIndex};
use crate::shorthand::{
    self, expand_four_sides, BorderState, ALL_SIDES, SIDE_BOTTOM, SIDE_LEFT, SIDE_RIGHT, SIDE_TOP,
};
use crate::sibling_index::{SiblingIndex, SiblingPos};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StyleContext {
    pub viewport_width: f32,
    pub viewport_height: f32,
    pub hovered_node: Option<NodeId>,
    pub active_node: Option<NodeId>,
    pub focused_node: Option<NodeId>,
}

impl Default for StyleContext {
    fn default() -> Self {
        Self {
            viewport_width: 1024.0,
            viewport_height: 768.0,
            hovered_node: None,
            active_node: None,
            focused_node: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct StyledNode {
    pub node_id: NodeId,
    pub style: ComputedStyle,
    pub href: Option<String>,
    pub children: Vec<StyledNode>,
}

const UA_CSS: &str = r#"
head, script, style, title, meta, link, noscript, template {
    display: none;
}

html, body, div, p, h1, h2, h3, h4, h5, h6,
ul, ol, li, pre, hr, section, article, header, footer, main, nav, blockquote {
    display: block;
}

body {
    margin: 0;
    padding: 16px;
    color: #0f172a;
    background-color: #ffffff;
}

p {
    margin: 10px 0;
}

h1 {
    font-size: 32px;
    font-weight: bold;
    margin: 16px 0 12px 0;
}

h2 {
    font-size: 24px;
    font-weight: bold;
    margin: 14px 0 10px 0;
}

h3 {
    font-size: 20px;
    font-weight: bold;
    margin: 12px 0 8px 0;
}

h4 {
    font-size: 18px;
    font-weight: bold;
    margin: 10px 0 6px 0;
}

h5 {
    font-size: 16px;
    font-weight: bold;
    margin: 8px 0 4px 0;
}

h6 {
    font-size: 14px;
    font-weight: bold;
    margin: 8px 0 4px 0;
}

strong, b {
    font-weight: bold;
}

u {
    text-decoration: underline;
}

a {
    color: #2563eb;
    text-decoration: underline;
}

pre, code, kbd, samp {
    font-family: monospace;
}

pre {
    display: block;
    white-space: pre;
    padding: 12px;
    margin: 10px 0;
    background-color: #f1f5f9;
}

code {
    background-color: #f1f5f9;
    color: #0f172a;
    padding: 2px 4px;
}

pre code {
    padding: 0;
}

img {
    display: inline-block;
}

input, textarea, select, button {
    display: inline-block;
    font-size: 14px;
    color: #0f172a;
}

textarea, select {
    background-color: #ffffff;
}

textarea, select, button {
    padding: 5px 8px;
    border: 1px solid #94a3b8;
    border-radius: 6px;
}

textarea {
    width: 240px;
    height: 64px;
}

button {
    padding: 6px 12px;
    background-color: #f8fafc;
}

input:focus, textarea:focus, select:focus, button:focus {
    border-color: #2563eb;
}

hr {
    display: block;
    height: 1px;
    background-color: #cbd5e1;
    margin: 12px 0;
}

ul, ol {
    display: block;
    margin: 8px 0;
    padding-left: 24px;
}

li {
    display: block;
    margin: 4px 0;
}

blockquote {
    display: block;
    margin: 10px 0;
    padding: 6px 12px;
    border-left: 3px solid #cbd5e1;
}
"#;

fn ua_stylesheet_ref() -> &'static Stylesheet {
    static UA_SHEET: OnceLock<Stylesheet> = OnceLock::new();
    UA_SHEET.get_or_init(|| css::parse(UA_CSS))
}

fn ua_index_ref() -> &'static RuleIndex {
    static UA_INDEX: OnceLock<RuleIndex> = OnceLock::new();
    UA_INDEX.get_or_init(|| RuleIndex::build(ua_stylesheet_ref()))
}

pub fn user_agent_stylesheet() -> Stylesheet {
    ua_stylesheet_ref().clone()
}

pub fn stylesheet_uses_interaction(css: &Stylesheet) -> bool {
    css.rules.iter().any(|rule| {
        rule.selectors.iter().any(|sel| {
            !sel.parts.iter().any(|(_, c)| c.unsupported_pseudo)
                && sel.parts.iter().any(|(_, c)| c.uses_interaction())
        })
    })
}

pub fn style_tree(doc: &Document, author_css: &Stylesheet) -> Option<StyledNode> {
    style_tree_with_context(doc, author_css, &StyleContext::default())
}

pub fn style_tree_with_context(
    doc: &Document,
    author_css: &Stylesheet,
    ctx: &StyleContext,
) -> Option<StyledNode> {
    let root_id = doc.root();
    doc.get(root_id)?;
    let cascade = CascadeContext {
        doc,
        ua_css: ua_stylesheet_ref(),
        ua_index: ua_index_ref(),
        author_css,
        author_index: RuleIndex::build(author_css),
        candidates: RefCell::new(Vec::new()),
        bloom: RefCell::new(AncestorBloom::new()),
        siblings: RefCell::new(SiblingIndex::default()),
        style_ctx: ctx,
        hovered_ancestors: collect_ancestors(doc, ctx.hovered_node),
        active_ancestors: collect_ancestors(doc, ctx.active_node),
    };
    let root_style = ComputedStyle {
        display: Display::Block,
        background_color: Some(Color::rgb(255, 255, 255)),
        ..ComputedStyle::default()
    };
    build_styled_node(&cascade, root_id, &root_style, 16.0, None)
}

struct CascadeContext<'a> {
    doc: &'a Document,
    ua_css: &'a Stylesheet,
    ua_index: &'a RuleIndex,
    author_css: &'a Stylesheet,
    /// Built once per style pass (rules are public and may change between passes).
    author_index: RuleIndex,
    /// Scratch buffer reused for every element's candidate rule list.
    candidates: RefCell<Vec<u32>>,
    /// Keys of the ancestors of the element currently being styled (maintained by
    /// `build_styled_node`'s depth-first walk).
    bloom: RefCell<AncestorBloom>,
    /// Sibling positions for structural selectors, filled per parent on first use.
    siblings: RefCell<SiblingIndex>,
    style_ctx: &'a StyleContext,
    hovered_ancestors: Vec<NodeId>,
    active_ancestors: Vec<NodeId>,
}

pub(crate) struct LengthEnv<'a> {
    pub(crate) em: f32,
    pub(crate) rem: f32,
    pub(crate) ctx: &'a StyleContext,
}

fn collect_ancestors(doc: &Document, start: Option<NodeId>) -> Vec<NodeId> {
    let mut ancestors = Vec::new();
    let mut curr = start;
    while let Some(id) = curr {
        if ancestors.contains(&id) {
            break;
        }
        let Some(node) = doc.get(id) else {
            break;
        };
        ancestors.push(id);
        curr = node.parent;
    }
    ancestors
}

fn build_styled_node(
    cascade: &CascadeContext<'_>,
    node_id: NodeId,
    parent_style: &ComputedStyle,
    root_font_size: f32,
    inherited_href: Option<&str>,
) -> Option<StyledNode> {
    let node = cascade.doc.get(node_id)?;
    match &node.kind {
        NodeKind::Document => {
            let style = ComputedStyle {
                display: Display::Block,
                background_color: Some(Color::rgb(255, 255, 255)),
                ..ComputedStyle::default()
            };
            let mut children = Vec::new();
            for &child_id in &node.children {
                if let Some(child) =
                    build_styled_node(cascade, child_id, &style, root_font_size, inherited_href)
                {
                    children.push(child);
                }
            }
            Some(StyledNode {
                node_id,
                style,
                href: None,
                children,
            })
        }
        NodeKind::Text(_) => {
            let style = inherit_for_text(parent_style);
            Some(StyledNode {
                node_id,
                style,
                href: inherited_href.map(str::to_string),
                children: Vec::new(),
            })
        }
        NodeKind::Element(elem) => {
            let style =
                compute_element_style(cascade, node_id, elem, parent_style, root_font_size);
            if style.display == Display::None {
                return None;
            }

            let next_root_font_size = if elem.tag_name.eq_ignore_ascii_case("html") {
                style.font_size
            } else {
                root_font_size
            };

            let node_href: Option<String> = if elem.tag_name.eq_ignore_ascii_case("a") {
                elem.attr("href")
                    .map(str::to_string)
                    .or_else(|| inherited_href.map(str::to_string))
            } else {
                inherited_href.map(str::to_string)
            };

            let mut children = Vec::new();
            cascade.bloom.borrow_mut().push_element(elem);
            for &child_id in &node.children {
                if let Some(child) = build_styled_node(
                    cascade,
                    child_id,
                    &style,
                    next_root_font_size,
                    node_href.as_deref(),
                ) {
                    children.push(child);
                }
            }
            cascade.bloom.borrow_mut().pop_element(elem);

            Some(StyledNode {
                node_id,
                style,
                href: node_href,
                children,
            })
        }
    }
}

fn inherit_for_text(parent: &ComputedStyle) -> ComputedStyle {
    ComputedStyle {
        color: parent.color,
        border_color: parent.color,
        font_size: parent.font_size,
        bold: parent.bold,
        monospace: parent.monospace,
        text_underline: parent.text_underline,
        preformatted: parent.preformatted,
        ..ComputedStyle::default()
    }
}

fn compute_element_style(
    cascade: &CascadeContext<'_>,
    node_id: NodeId,
    elem: &ElementData,
    parent_style: &ComputedStyle,
    root_font_size: f32,
) -> ComputedStyle {
    let mut style = ComputedStyle {
        color: parent_style.color,
        border_color: parent_style.color,
        font_size: parent_style.font_size,
        bold: parent_style.bold,
        monospace: parent_style.monospace,
        text_underline: parent_style.text_underline,
        preformatted: parent_style.preformatted,
        ..ComputedStyle::default()
    };

    let ua_matches = matching_rules(cascade, node_id, elem, cascade.ua_css, cascade.ua_index);
    let author_matches = matching_rules(
        cascade,
        node_id,
        elem,
        cascade.author_css,
        &cascade.author_index,
    );
    let inline_decls = elem
        .attr("style")
        .map(css::parse_inline_style)
        .unwrap_or_default();

    // Pass 1: resolve font-size first so `em` units on other properties use this element's font-size.
    for decl in ua_matches
        .iter()
        .chain(author_matches.iter())
        .flat_map(|(_, _, decls)| decls.iter())
        .chain(inline_decls.iter())
    {
        let size_value = match decl.name.as_str() {
            "font-size" => decl.first(),
            "font" => shorthand::parse_font_shorthand(&decl.values).map(|f| f.size),
            _ => None,
        };
        if let Some(fs) = size_value.and_then(|v| {
            resolve_font_size(v, parent_style.font_size, root_font_size, cascade.style_ctx)
        }) {
            style.font_size = fs;
        }
    }

    // Pass 2: apply all declarations in cascade order.
    let effective_rem = if elem.tag_name.eq_ignore_ascii_case("html") {
        style.font_size
    } else {
        root_font_size
    };
    let env = LengthEnv {
        em: style.font_size,
        rem: effective_rem,
        ctx: cascade.style_ctx,
    };
    let mut border = BorderState::new();
    for decl in ua_matches
        .iter()
        .chain(author_matches.iter())
        .flat_map(|(_, _, decls)| decls.iter())
        .chain(inline_decls.iter())
    {
        if decl.name == "box-sizing" {
            shorthand::apply_box_sizing(&mut style, &decl.values, parent_style.box_sizing);
        } else {
            apply_declaration(&mut style, decl, &env, &mut border);
        }
    }
    border.finish(&mut style);

    style
}

/// Rules of `stylesheet` matching `node_id`, sorted by (specificity, source order).
/// Only rules from the element's index buckets are tested (see `rule_index`).
fn matching_rules<'a>(
    cascade: &CascadeContext<'_>,
    node_id: NodeId,
    elem: &ElementData,
    stylesheet: &'a Stylesheet,
    index: &RuleIndex,
) -> Vec<(Specificity, usize, &'a [Declaration])> {
    let mut candidates = cascade.candidates.borrow_mut();
    let is_root = || {
        cascade
            .doc
            .get(node_id)
            .and_then(|n| n.parent)
            .and_then(|p| cascade.doc.get(p))
            .is_some_and(|p| matches!(p.kind, NodeKind::Document))
    };
    index.candidates(elem, is_root, &mut candidates);
    let bloom = cascade.bloom.borrow();
    let mut matched = Vec::new();
    for &idx in candidates.iter() {
        let idx = idx as usize;
        if let Some(rule) = stylesheet.rules.get(idx) {
            let bloom_reject =
                |sel_idx: usize| !bloom.may_match(&index.ancestor_hashes(idx, sel_idx));
            push_if_matches(cascade, node_id, idx, rule, bloom_reject, &mut matched);
        }
    }
    matched.sort_by_key(|&(spec, idx, _)| (spec, idx));
    matched
}

/// `skip(selector_index)` lets the caller reject a selector cheaply (ancestor Bloom filter).
fn push_if_matches<'a>(
    cascade: &CascadeContext<'_>,
    node_id: NodeId,
    idx: usize,
    rule: &'a css::Rule,
    skip: impl Fn(usize) -> bool,
    matched: &mut Vec<(Specificity, usize, &'a [Declaration])>,
) {
    let mut best_spec: Option<Specificity> = None;
    for (sel_idx, sel) in rule.selectors.iter().enumerate() {
        if !skip(sel_idx) && matches_complex(cascade, node_id, sel) {
            let spec = sel.specificity();
            best_spec = Some(match best_spec {
                Some(prev) if prev >= spec => prev,
                _ => spec,
            });
        }
    }
    if let Some(spec) = best_spec {
        matched.push((spec, idx, rule.declarations.as_slice()));
    }
}

/// Reference implementation: tests every rule. Used to prove the index changes nothing.
#[cfg(test)]
fn matching_rules_all<'a>(
    cascade: &CascadeContext<'_>,
    node_id: NodeId,
    stylesheet: &'a Stylesheet,
) -> Vec<(Specificity, usize, &'a [Declaration])> {
    let mut matched = Vec::new();
    for (idx, rule) in stylesheet.rules.iter().enumerate() {
        push_if_matches(cascade, node_id, idx, rule, |_| false, &mut matched);
    }
    matched.sort_by_key(|&(spec, idx, _)| (spec, idx));
    matched
}

fn matches_complex(
    cascade: &CascadeContext<'_>,
    node_id: NodeId,
    sel: &ComplexSelector,
) -> bool {
    matches_parts_at(cascade, node_id, &sel.parts)
}

fn matches_parts_at(
    cascade: &CascadeContext<'_>,
    node_id: NodeId,
    parts: &[(Option<Combinator>, CompoundSelector)],
) -> bool {
    let Some((combinator_to_prev, compound)) = parts.last() else {
        return false;
    };
    let Some(elem) = cascade.doc.element(node_id) else {
        return false;
    };
    if !matches_compound(cascade, node_id, elem, compound) {
        return false;
    }
    if parts.len() == 1 {
        return true;
    }
    let prefix = &parts[..parts.len() - 1];
    match combinator_to_prev {
        Some(Combinator::Child) => {
            let Some(parent_id) = cascade.doc.get(node_id).and_then(|n| n.parent) else {
                return false;
            };
            matches_parts_at(cascade, parent_id, prefix)
        }
        Some(Combinator::Descendant) => {
            let mut anc = cascade.doc.get(node_id).and_then(|n| n.parent);
            while let Some(anc_id) = anc {
                if matches_parts_at(cascade, anc_id, prefix) {
                    return true;
                }
                anc = cascade.doc.get(anc_id).and_then(|n| n.parent);
            }
            false
        }
        Some(Combinator::NextSibling) => cascade
            .prev_element_sibling(node_id)
            .is_some_and(|prev| matches_parts_at(cascade, prev, prefix)),
        Some(Combinator::SubsequentSibling) => {
            let mut prev = cascade.prev_element_sibling(node_id);
            while let Some(prev_id) = prev {
                if matches_parts_at(cascade, prev_id, prefix) {
                    return true;
                }
                prev = cascade.prev_element_sibling(prev_id);
            }
            false
        }
        None => false,
    }
}

fn matches_compound(
    cascade: &CascadeContext<'_>,
    node_id: NodeId,
    elem: &ElementData,
    sel: &CompoundSelector,
) -> bool {
    if sel.unsupported_pseudo {
        return false;
    }
    if !sel.universal
        && sel.tag_name.is_none()
        && sel.id.is_none()
        && sel.classes.is_empty()
        && !sel.hover
        && !sel.active
        && !sel.focus
        && sel.attrs.is_empty()
        && sel.pseudo.is_empty()
    {
        return false;
    }
    if let Some(ref tag) = sel.tag_name {
        if !elem.tag_name.eq_ignore_ascii_case(tag) {
            return false;
        }
    }
    if let Some(ref id) = sel.id {
        if elem.id() != Some(id.as_str()) {
            return false;
        }
    }
    for cls in &sel.classes {
        if !elem.has_class(cls) {
            return false;
        }
    }
    if sel.hover && !cascade.hovered_ancestors.contains(&node_id) {
        return false;
    }
    if sel.active && !cascade.active_ancestors.contains(&node_id) {
        return false;
    }
    if sel.focus && cascade.style_ctx.focused_node != Some(node_id) {
        return false;
    }
    for attr in &sel.attrs {
        if !elem.attr(&attr.name).is_some_and(|v| attr.matches(v)) {
            return false;
        }
    }
    sel.pseudo
        .iter()
        .all(|p| matches_pseudo(cascade, node_id, p))
}

impl CascadeContext<'_> {
    fn sibling_pos(&self, node_id: NodeId) -> SiblingPos {
        self.siblings.borrow_mut().get(self.doc, node_id)
    }

    fn prev_element_sibling(&self, node_id: NodeId) -> Option<NodeId> {
        self.siblings
            .borrow_mut()
            .prev_element_sibling(self.doc, node_id)
    }
}

fn matches_any(cascade: &CascadeContext<'_>, node_id: NodeId, list: &[ComplexSelector]) -> bool {
    list.iter().any(|sel| matches_complex(cascade, node_id, sel))
}

fn matches_pseudo(
    cascade: &CascadeContext<'_>,
    node_id: NodeId,
    pseudo: &PseudoClass,
) -> bool {
    let doc = cascade.doc;
    let parent = doc.get(node_id).and_then(|n| n.parent);
    let pos = || cascade.sibling_pos(node_id);
    match pseudo {
        PseudoClass::Root => parent
            .and_then(|p| doc.get(p))
            .is_some_and(|p| matches!(p.kind, NodeKind::Document)),
        PseudoClass::Empty => doc.get(node_id).is_some_and(|n| {
            n.children.iter().all(|&c| {
                matches!(doc.get(c).map(|c| &c.kind), Some(NodeKind::Text(t)) if t.is_empty())
            })
        }),
        PseudoClass::NthChild { nth, last, of } if of.is_empty() => {
            let p = pos();
            if p.elem_index == 0 {
                return false;
            }
            let index = if *last {
                p.elem_count - p.elem_index + 1
            } else {
                p.elem_index
            };
            nth.matches(index as usize)
        }
        PseudoClass::NthChild { nth, last, of } => {
            // `:nth-child(An+B of S)`: position among siblings matching S. Rare; O(siblings).
            if !matches_any(cascade, node_id, of) {
                return false;
            }
            let Some(siblings) = parent.and_then(|p| doc.get(p)).map(|p| &p.children) else {
                return false;
            };
            let Some(me) = siblings.iter().position(|&s| s == node_id) else {
                return false;
            };
            let range = if *last { &siblings[me + 1..] } else { &siblings[..me] };
            let before = range
                .iter()
                .filter(|&&s| doc.element(s).is_some() && matches_any(cascade, s, of))
                .count();
            nth.matches(before + 1)
        }
        PseudoClass::NthOfType { nth, last } => {
            let p = pos();
            if p.type_index == 0 {
                return false;
            }
            let index = if *last {
                p.type_count - p.type_index + 1
            } else {
                p.type_index
            };
            nth.matches(index as usize)
        }
        PseudoClass::OnlyChild => pos().elem_count == 1,
        PseudoClass::OnlyOfType => pos().type_count == 1,
        PseudoClass::Not(list) => !matches_any(cascade, node_id, list),
        PseudoClass::Is(list) | PseudoClass::Where(list) => matches_any(cascade, node_id, list),
    }
}

fn resolve_font_size(
    val: &Value,
    parent_font_size: f32,
    root_font_size: f32,
    ctx: &StyleContext,
) -> Option<f32> {
    let px = match val {
        Value::Length(n, Unit::Px) => *n,
        Value::Length(n, Unit::Em) => parent_font_size * n,
        Value::Length(n, Unit::Rem) => root_font_size * n,
        Value::Length(n, Unit::Percent) => parent_font_size * (n / 100.0),
        Value::Length(n, Unit::Vh) => (ctx.viewport_height.max(0.0) / 100.0) * n,
        Value::Length(n, Unit::Vw) => (ctx.viewport_width.max(0.0) / 100.0) * n,
        Value::Number(n) => *n,
        Value::Keyword(kw) => match kw.as_str() {
            "xx-small" => 9.0,
            "x-small" => 10.0,
            "small" => 13.0,
            "medium" => 16.0,
            "large" => 18.0,
            "x-large" => 24.0,
            "xx-large" => 32.0,
            "xxx-large" => 48.0,
            "smaller" => parent_font_size * 0.85,
            "larger" => parent_font_size * 1.2,
            _ => return None,
        },
        Value::Color(_) => return None,
    };
    Some(px.max(1.0))
}

pub(crate) fn resolve_length(val: &Value, env: &LengthEnv<'_>) -> Option<f32> {
    match val {
        Value::Length(n, Unit::Px) => Some(*n),
        Value::Length(n, Unit::Em) => Some(n * env.em),
        Value::Length(n, Unit::Rem) => Some(n * env.rem),
        Value::Length(n, Unit::Vh) => Some(n * (env.ctx.viewport_height.max(0.0) / 100.0)),
        Value::Length(n, Unit::Vw) => Some(n * (env.ctx.viewport_width.max(0.0) / 100.0)),
        Value::Number(n) if *n == 0.0 => Some(0.0),
        _ => None,
    }
}

fn parse_non_neg_number(val: &Value) -> Option<f32> {
    match val {
        Value::Number(n) if *n >= 0.0 => Some(*n),
        Value::Length(n, Unit::Px) if *n == 0.0 => Some(0.0),
        _ => None,
    }
}

pub(crate) fn is_monospace_font(lower: &str) -> bool {
    if matches!(
        lower,
        "monospace"
            | "ui-monospace"
            | "courier"
            | "courier new"
            | "consolas"
            | "menlo"
            | "monaco"
            | "inconsolata"
            | "hack"
            | "fira code"
            | "source code pro"
            | "cascadia code"
            | "cascadia mono"
    ) {
        return true;
    }
    !lower.contains("monotype")
        && lower
            .split(|c: char| !c.is_ascii_alphanumeric())
            .any(|w| matches!(w, "mono" | "monospace" | "sfmono"))
}

fn apply_declaration(
    style: &mut ComputedStyle,
    decl: &Declaration,
    env: &LengthEnv<'_>,
    border: &mut BorderState,
) {
    let values = decl.values.as_slice();
    match decl.name.as_str() {
        "display" => {
            if let Some(Value::Keyword(kw)) = decl.first() {
                match kw.as_str() {
                    "block" => style.display = Display::Block,
                    "inline" => style.display = Display::Inline,
                    "inline-block" => style.display = Display::InlineBlock,
                    "flex" => style.display = Display::Flex,
                    "none" => style.display = Display::None,
                    _ => {}
                }
            }
        }
        "color" => {
            if let Some(Value::Color(c)) = decl.first() {
                style.color = *c;
            }
        }
        "background-color" => {
            shorthand::apply_background_color(&mut style.background_color, values)
        }
        "background" => shorthand::apply_background(&mut style.background_color, values),
        "margin" => apply_margin_shorthand(style, &decl.values, env),
        "margin-top" => {
            if let Some(v) = decl.first().and_then(|v| resolve_length(v, env)) {
                style.margin.top = v;
            }
        }
        "margin-right" => {
            if let Some(val) = decl.first() {
                if matches!(val, Value::Keyword(k) if k == "auto") {
                    style.margin.right = 0.0;
                    style.margin_right_auto = true;
                } else if let Some(v) = resolve_length(val, env) {
                    style.margin.right = v;
                    style.margin_right_auto = false;
                }
            }
        }
        "margin-bottom" => {
            if let Some(v) = decl.first().and_then(|v| resolve_length(v, env)) {
                style.margin.bottom = v;
            }
        }
        "margin-left" => {
            if let Some(val) = decl.first() {
                if matches!(val, Value::Keyword(k) if k == "auto") {
                    style.margin.left = 0.0;
                    style.margin_left_auto = true;
                } else if let Some(v) = resolve_length(val, env) {
                    style.margin.left = v;
                    style.margin_left_auto = false;
                }
            }
        }
        "padding" => shorthand::apply_padding(&mut style.padding, values, env),
        "padding-top" => shorthand::apply_padding_side(&mut style.padding.top, values, env),
        "padding-right" => shorthand::apply_padding_side(&mut style.padding.right, values, env),
        "padding-bottom" => shorthand::apply_padding_side(&mut style.padding.bottom, values, env),
        "padding-left" => shorthand::apply_padding_side(&mut style.padding.left, values, env),
        "border" => shorthand::apply_border(style, border, ALL_SIDES, values, env),
        "border-top" => shorthand::apply_border(style, border, SIDE_TOP, values, env),
        "border-right" => shorthand::apply_border(style, border, SIDE_RIGHT, values, env),
        "border-bottom" => shorthand::apply_border(style, border, SIDE_BOTTOM, values, env),
        "border-left" => shorthand::apply_border(style, border, SIDE_LEFT, values, env),
        "border-width" => shorthand::apply_border_width(&mut style.border, values, env),
        "border-top-width" => {
            shorthand::apply_border_side_width(&mut style.border.top, values, env)
        }
        "border-right-width" => {
            shorthand::apply_border_side_width(&mut style.border.right, values, env)
        }
        "border-bottom-width" => {
            shorthand::apply_border_side_width(&mut style.border.bottom, values, env)
        }
        "border-left-width" => {
            shorthand::apply_border_side_width(&mut style.border.left, values, env)
        }
        "border-style" => shorthand::apply_border_style(border, ALL_SIDES, values),
        "border-top-style" => shorthand::apply_border_style(border, SIDE_TOP, values),
        "border-right-style" => shorthand::apply_border_style(border, SIDE_RIGHT, values),
        "border-bottom-style" => shorthand::apply_border_style(border, SIDE_BOTTOM, values),
        "border-left-style" => shorthand::apply_border_style(border, SIDE_LEFT, values),
        "border-color" => shorthand::apply_border_color(style, border, values),
        "border-radius" => {
            if let Some(v) = decl.first().and_then(|v| resolve_length(v, env)) {
                style.border_radius = v.max(0.0);
            }
        }
        "width" => shorthand::apply_size(&mut style.width, values, env),
        "max-width" => shorthand::apply_max_size(&mut style.max_width, values, env),
        "min-width" => shorthand::apply_min_size(&mut style.min_width, values, env),
        "min-height" => shorthand::apply_min_size(&mut style.min_height, values, env),
        "max-height" => shorthand::apply_max_size(&mut style.max_height, values, env),
        "inset" => shorthand::apply_inset(style, values, env),
        "top" => shorthand::apply_inset_side(&mut style.inset.top, values, env),
        "right" => shorthand::apply_inset_side(&mut style.inset.right, values, env),
        "bottom" => shorthand::apply_inset_side(&mut style.inset.bottom, values, env),
        "left" => shorthand::apply_inset_side(&mut style.inset.left, values, env),
        "font" => {
            if let Some(font) = shorthand::parse_font_shorthand(values) {
                style.bold = font.bold;
                style.monospace = font.monospace;
            }
        }
        "height" => shorthand::apply_size(&mut style.height, values, env),
        "font-weight" => {
            if let Some(val) = decl.first() {
                match val {
                    Value::Keyword(kw) => match kw.as_str() {
                        "bold" | "bolder" => style.bold = true,
                        "normal" | "lighter" => style.bold = false,
                        _ => {}
                    },
                    Value::Number(n) => style.bold = *n >= 600.0,
                    Value::Length(n, Unit::Px) if *n == 0.0 => style.bold = false,
                    _ => {}
                }
            }
        }
        "font-family" => {
            let mut has_font_token = false;
            let mut is_mono = false;
            let mut is_inherit = false;
            for val in &decl.values {
                if let Value::Keyword(kw) = val {
                    let lower = kw.to_ascii_lowercase();
                    if lower == "inherit" || lower == "unset" {
                        is_inherit = true;
                        continue;
                    }
                    if lower == "initial" {
                        has_font_token = true;
                        is_mono = false;
                        continue;
                    }
                    has_font_token = true;
                    if is_monospace_font(&lower) {
                        is_mono = true;
                    }
                }
            }
            if has_font_token && !is_inherit {
                style.monospace = is_mono;
            }
        }
        "text-decoration" | "text-decoration-line" => {
            let mut has_underline = false;
            let mut has_none = false;
            for val in &decl.values {
                if let Value::Keyword(kw) = val {
                    if kw == "underline" {
                        has_underline = true;
                    } else if kw == "none" {
                        has_none = true;
                    }
                }
            }
            if has_underline {
                style.text_underline = true;
            } else if has_none {
                style.text_underline = false;
            }
        }
        "white-space" => {
            if let Some(Value::Keyword(kw)) = decl.first() {
                match kw.as_str() {
                    "pre" | "pre-wrap" | "pre-line" => style.preformatted = true,
                    "normal" | "nowrap" => style.preformatted = false,
                    _ => {}
                }
            }
        }
        "flex-direction" => {
            if let Some(Value::Keyword(kw)) = decl.first() {
                match kw.as_str() {
                    "row" => style.flex_direction = FlexDirection::Row,
                    "column" => style.flex_direction = FlexDirection::Column,
                    "row-reverse" => style.flex_direction = FlexDirection::RowReverse,
                    "column-reverse" => style.flex_direction = FlexDirection::ColumnReverse,
                    _ => {}
                }
            }
        }
        "flex-wrap" => {
            if let Some(Value::Keyword(kw)) = decl.first() {
                match kw.as_str() {
                    "nowrap" => style.flex_wrap = FlexWrap::NoWrap,
                    "wrap" => style.flex_wrap = FlexWrap::Wrap,
                    "wrap-reverse" => style.flex_wrap = FlexWrap::WrapReverse,
                    _ => {}
                }
            }
        }
        "justify-content" => {
            if let Some(Value::Keyword(kw)) = decl.first() {
                match kw.as_str() {
                    "flex-start" | "start" => style.justify_content = JustifyContent::FlexStart,
                    "flex-end" | "end" => style.justify_content = JustifyContent::FlexEnd,
                    "center" => style.justify_content = JustifyContent::Center,
                    "space-between" => style.justify_content = JustifyContent::SpaceBetween,
                    "space-around" => style.justify_content = JustifyContent::SpaceAround,
                    "space-evenly" => style.justify_content = JustifyContent::SpaceEvenly,
                    _ => {}
                }
            }
        }
        "align-items" => {
            if let Some(Value::Keyword(kw)) = decl.first() {
                match kw.as_str() {
                    "stretch" => style.align_items = AlignItems::Stretch,
                    "flex-start" | "start" => style.align_items = AlignItems::FlexStart,
                    "flex-end" | "end" => style.align_items = AlignItems::FlexEnd,
                    "center" => style.align_items = AlignItems::Center,
                    "baseline" => style.align_items = AlignItems::Baseline,
                    _ => {}
                }
            }
        }
        "gap" => {
            if let Some(v) = decl.first().and_then(|v| resolve_length(v, env)) {
                style.gap = v.max(0.0);
            }
        }
        "flex-grow" => {
            if let Some(g) = decl.first().and_then(parse_non_neg_number) {
                style.flex_grow = g;
            }
        }
        "flex-shrink" => {
            if let Some(s) = decl.first().and_then(parse_non_neg_number) {
                style.flex_shrink = s;
            }
        }
        "flex-basis" => {
            if let Some(basis) = decl.first().and_then(|v| resolve_flex_basis(v, env)) {
                style.flex_basis = basis;
            }
        }
        "flex" => apply_flex_shorthand(style, &decl.values, env),
        _ => {}
    }
}

fn apply_margin_shorthand(style: &mut ComputedStyle, values: &[Value], env: &LengthEnv<'_>) {
    #[derive(Clone, Copy)]
    enum MarginVal {
        Px(f32),
        Auto,
    }

    let parsed: Option<Vec<MarginVal>> = values
        .iter()
        .map(|v| {
            if matches!(v, Value::Keyword(k) if k == "auto") {
                Some(MarginVal::Auto)
            } else {
                resolve_length(v, env).map(MarginVal::Px)
            }
        })
        .collect();

    let Some(parsed) = parsed else {
        return;
    };
    let Some((t, r, b, l)) = expand_four_sides(&parsed) else {
        return;
    };

    style.margin.top = match t {
        MarginVal::Px(n) => n,
        MarginVal::Auto => 0.0,
    };
    style.margin.bottom = match b {
        MarginVal::Px(n) => n,
        MarginVal::Auto => 0.0,
    };
    match r {
        MarginVal::Px(n) => {
            style.margin.right = n;
            style.margin_right_auto = false;
        }
        MarginVal::Auto => {
            style.margin.right = 0.0;
            style.margin_right_auto = true;
        }
    }
    match l {
        MarginVal::Px(n) => {
            style.margin.left = n;
            style.margin_left_auto = false;
        }
        MarginVal::Auto => {
            style.margin.left = 0.0;
            style.margin_left_auto = true;
        }
    }
}

fn resolve_flex_basis(val: &Value, env: &LengthEnv<'_>) -> Option<FlexBasis> {
    match val {
        Value::Keyword(k) if k == "auto" => Some(FlexBasis::Auto),
        Value::Length(pct, Unit::Percent) => Some(FlexBasis::Percent((*pct).max(0.0))),
        _ => resolve_length(val, env).map(|px| FlexBasis::Px(px.max(0.0))),
    }
}

fn apply_flex_shorthand(style: &mut ComputedStyle, values: &[Value], env: &LengthEnv<'_>) {
    match values {
        [single] => match single {
            Value::Keyword(k) if k == "none" => {
                style.flex_grow = 0.0;
                style.flex_shrink = 0.0;
                style.flex_basis = FlexBasis::Auto;
            }
            Value::Keyword(k) if k == "auto" => {
                style.flex_grow = 1.0;
                style.flex_shrink = 1.0;
                style.flex_basis = FlexBasis::Auto;
            }
            Value::Keyword(k) if k == "initial" => {
                style.flex_grow = 0.0;
                style.flex_shrink = 1.0;
                style.flex_basis = FlexBasis::Auto;
            }
            Value::Number(g) if *g >= 0.0 => {
                style.flex_grow = *g;
                style.flex_shrink = 1.0;
                style.flex_basis = FlexBasis::Percent(0.0);
            }
            Value::Length(n, Unit::Px) if *n == 0.0 => {
                style.flex_grow = 0.0;
                style.flex_shrink = 1.0;
                style.flex_basis = FlexBasis::Percent(0.0);
            }
            other => {
                if let Some(basis) = resolve_flex_basis(other, env) {
                    style.flex_grow = 1.0;
                    style.flex_shrink = 1.0;
                    style.flex_basis = basis;
                }
            }
        },
        [first, second] => {
            let Some(g) = parse_non_neg_number(first) else {
                return;
            };
            match second {
                Value::Number(s) if *s >= 0.0 => {
                    style.flex_grow = g;
                    style.flex_shrink = *s;
                    style.flex_basis = FlexBasis::Percent(0.0);
                }
                Value::Length(n, Unit::Px) if *n == 0.0 => {
                    style.flex_grow = g;
                    style.flex_shrink = 0.0;
                    style.flex_basis = FlexBasis::Percent(0.0);
                }
                other => {
                    if let Some(basis) = resolve_flex_basis(other, env) {
                        style.flex_grow = g;
                        style.flex_shrink = 1.0;
                        style.flex_basis = basis;
                    }
                }
            }
        }
        [first, second, third] => {
            let (Some(g), Some(s), Some(basis)) = (
                parse_non_neg_number(first),
                parse_non_neg_number(second),
                resolve_flex_basis(third, env),
            ) else {
                return;
            };
            style.flex_grow = g;
            style.flex_shrink = s;
            style.flex_basis = basis;
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::computed::{EdgeSizes, LengthPct};

    fn attrs(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn test_computed_style_is_copy() {
        fn assert_copy<T: Copy>() {}
        assert_copy::<ComputedStyle>();
        assert_copy::<StyleContext>();
        assert_copy::<Display>();
        assert_copy::<FlexDirection>();
        assert_copy::<FlexWrap>();
        assert_copy::<JustifyContent>();
        assert_copy::<AlignItems>();
        assert_copy::<FlexBasis>();
        assert_copy::<EdgeSizes>();
    }

    #[test]
    fn test_specificity_cascade_and_inline_override() {
        let mut doc = Document::new();
        let body = doc.create_element("body", Vec::new());
        let card = doc.create_element(
            "div",
            attrs(&[
                ("id", "main"),
                ("class", "card highlight"),
                ("style", "color: #112233;"),
            ]),
        );
        let p = doc.create_element("p", attrs(&[("class", "lead")]));
        let txt = doc.create_text("Hello");
        doc.append_child(doc.root(), body);
        doc.append_child(body, card);
        doc.append_child(card, p);
        doc.append_child(p, txt);

        let css = css::parse(
            r#"
            #main { color: red; background-color: #eeeeee; font-size: 20px; }
            div.card { color: green; padding: 10px 20px; }
            .card.highlight { color: blue; }
            #main > p.lead { margin: 0 auto; font-weight: 700; }
            div p { font-weight: normal; }
            "#,
        );

        let tree = style_tree(&doc, &css).expect("styled tree");
        let styled_body = &tree.children[0];
        let styled_card = &styled_body.children[0];
        // Inline style overrides #main
        assert_eq!(styled_card.style.color, Color::rgb(0x11, 0x22, 0x33));
        assert_eq!(
            styled_card.style.background_color,
            Some(Color::rgb(0xee, 0xee, 0xee))
        );
        assert_eq!(styled_card.style.padding.top, 10.0);
        assert_eq!(styled_card.style.padding.left, 20.0);

        let styled_p = &styled_card.children[0];
        // #main > p.lead beats div p
        assert!(styled_p.style.bold);
        assert!(styled_p.style.margin_left_auto);
        assert!(styled_p.style.margin_right_auto);
        // Text inherits color, font_size, bold from p (not padding/bg)
        let styled_txt = &styled_p.children[0];
        assert_eq!(styled_txt.style.color, Color::rgb(0x11, 0x22, 0x33));
        assert_eq!(styled_txt.style.font_size, 20.0);
        assert!(styled_txt.style.bold);
    }

    #[test]
    fn test_combinators_descendant_vs_child_and_backtracking() {
        let mut doc = Document::new();
        let outer_div = doc.create_element("div", attrs(&[("class", "box")]));
        let outer_span = doc.create_element("span", Vec::new());
        let inner_div = doc.create_element("div", Vec::new());
        let inner_span = doc.create_element("span", Vec::new());
        let p = doc.create_element("p", Vec::new());
        doc.append_child(doc.root(), outer_div);
        doc.append_child(outer_div, outer_span);
        doc.append_child(outer_span, inner_div);
        doc.append_child(inner_div, inner_span);
        doc.append_child(inner_span, p);

        let css = css::parse(
            r#"
            .box > p { color: red; }
            .box > span p { color: teal; }
            "#,
        );

        let tree = style_tree(&doc, &css).unwrap();
        let p_node = &tree.children[0].children[0].children[0].children[0].children[0];
        // .box > p does NOT match, .box > span p DOES match via outer_span!
        assert_eq!(p_node.style.color, Color::rgb(0, 128, 128));
    }

    #[test]
    fn test_display_none_skipped_and_href_inherited() {
        let mut doc = Document::new();
        let head = doc.create_element("head", Vec::new());
        let title = doc.create_element("title", Vec::new());
        let title_txt = doc.create_text("Hidden");
        let body = doc.create_element("body", Vec::new());
        let a = doc.create_element("a", attrs(&[("href", "about:roadmap")]));
        let b = doc.create_element("b", Vec::new());
        let link_txt = doc.create_text("Roadmap");
        doc.append_child(doc.root(), head);
        doc.append_child(head, title);
        doc.append_child(title, title_txt);
        doc.append_child(doc.root(), body);
        doc.append_child(body, a);
        doc.append_child(a, b);
        doc.append_child(b, link_txt);

        let tree = style_tree(&doc, &Stylesheet::default()).unwrap();
        assert_eq!(tree.children.len(), 1); // head skipped
        let styled_a = &tree.children[0].children[0];
        assert_eq!(styled_a.href.as_deref(), Some("about:roadmap"));
        assert!(styled_a.style.text_underline);
        let styled_b = &styled_a.children[0];
        assert_eq!(styled_b.href.as_deref(), Some("about:roadmap"));
        assert!(styled_b.style.bold);
        assert!(styled_b.style.text_underline);
        let styled_txt = &styled_b.children[0];
        assert_eq!(styled_txt.href.as_deref(), Some("about:roadmap"));
        assert_eq!(styled_txt.style.color, Color::rgb(0x25, 0x63, 0xeb));
    }

    #[test]
    fn test_em_and_rem_resolution() {
        let mut doc = Document::new();
        let html = doc.create_element("html", attrs(&[("style", "font-size: 20px;")]));
        let div = doc.create_element(
            "div",
            attrs(&[("style", "font-size: 1.5em; padding: 1em; margin-top: 2rem;")]),
        );
        doc.append_child(doc.root(), html);
        doc.append_child(html, div);

        let tree = style_tree(&doc, &Stylesheet::default()).unwrap();
        let styled_div = &tree.children[0].children[0];
        assert_eq!(styled_div.style.font_size, 30.0);
        assert_eq!(styled_div.style.padding.top, 30.0); // 1em of self (30px)
        assert_eq!(styled_div.style.margin.top, 40.0); // 2rem of root html (20px)
    }

    #[test]
    fn test_nested_em_rem_vh_vw_and_percent_override() {
        let mut doc = Document::new();
        let html = doc.create_element("html", attrs(&[("style", "font-size: 20px;")]));
        let body = doc.create_element("body", attrs(&[("style", "font-size: 1.5em;")]));
        let box_el = doc.create_element(
            "div",
            attrs(&[(
                "style",
                "font-size: 2em; padding: 1em; margin-top: 2rem; width: 50vw; height: 25vh; border-radius: 0.5rem;",
            )]),
        );
        let pct_over_px = doc.create_element(
            "div",
            attrs(&[("style", "width: 200px; width: 50%; max-width: 80%; max-width: 640px; height: 40%; height: 120px;")]),
        );
        let px_over_pct = doc.create_element(
            "div",
            attrs(&[("style", "width: 50%; width: 180px;")]),
        );
        doc.append_child(doc.root(), html);
        doc.append_child(html, body);
        doc.append_child(body, box_el);
        doc.append_child(body, pct_over_px);
        doc.append_child(body, px_over_pct);

        let ctx = StyleContext {
            viewport_width: 1200.0,
            viewport_height: 800.0,
            ..StyleContext::default()
        };
        let tree = style_tree_with_context(&doc, &Stylesheet::default(), &ctx).unwrap();
        let styled_body = &tree.children[0].children[0];
        assert_eq!(styled_body.style.font_size, 30.0); // 1.5em of html (20px)

        let styled_box = &styled_body.children[0];
        assert_eq!(styled_box.style.font_size, 60.0); // 2em of body (30px)
        assert_eq!(styled_box.style.padding.left, 60.0); // 1em of self (60px)
        assert_eq!(styled_box.style.margin.top, 40.0); // 2rem of html (20px)
        assert_eq!(styled_box.style.width, LengthPct::Px(600.0)); // 50vw of 1200
        assert_eq!(styled_box.style.height, LengthPct::Px(200.0)); // 25vh of 800
        assert_eq!(styled_box.style.border_radius, 10.0); // 0.5rem of 20px

        let styled_pct_over_px = &styled_body.children[1];
        assert_eq!(styled_pct_over_px.style.width, LengthPct::Percent(50.0));
        assert_eq!(styled_pct_over_px.style.max_width, LengthPct::Px(640.0));
        assert_eq!(styled_pct_over_px.style.height, LengthPct::Px(120.0));

        let styled_px_over_pct = &styled_body.children[2];
        assert_eq!(styled_px_over_pct.style.width, LengthPct::Px(180.0));
    }

    #[test]
    fn test_hover_and_active_ancestor_propagation_and_unsupported_pseudo() {
        let mut doc = Document::new();
        let body = doc.create_element("body", Vec::new());
        let card = doc.create_element("div", attrs(&[("class", "card")]));
        let link = doc.create_element("a", attrs(&[("href", "about:test")]));
        let span = doc.create_element("span", Vec::new());
        let txt = doc.create_text("Click me");
        let sibling = doc.create_element("a", attrs(&[("href", "about:other")]));
        let sib_txt = doc.create_text("Other");

        doc.append_child(doc.root(), body);
        doc.append_child(body, card);
        doc.append_child(card, link);
        doc.append_child(link, span);
        doc.append_child(span, txt);
        doc.append_child(body, sibling);
        doc.append_child(sibling, sib_txt);

        let css = css::parse(
            r#"
            .card { background-color: #ffffff; }
            .card:hover { background-color: #f1f5f9; }
            a { color: #2563eb; }
            a:hover { color: #dc2626; }
            a:active { color: #16a34a; }
            a:nth-child(2) { color: #9333ea; }
            a::before { color: #f59e0b; }
            "#,
        );
        assert!(stylesheet_uses_interaction(&css));
        assert!(!stylesheet_uses_interaction(&css::parse("div { color: red; }")));

        let ctx = StyleContext {
            hovered_node: Some(txt),
            active_node: Some(span),
            ..StyleContext::default()
        };
        let tree = style_tree_with_context(&doc, &css, &ctx).unwrap();
        let styled_body = &tree.children[0];
        let styled_card = &styled_body.children[0];
        assert_eq!(
            styled_card.style.background_color,
            Some(Color::rgb(0xf1, 0xf5, 0xf9))
        );

        let styled_link = &styled_card.children[0];
        // Both :hover and :active match on `a`, :active comes later in source order at equal specificity (0,1,1)
        assert_eq!(styled_link.style.color, Color::rgb(0x16, 0xa3, 0x4a));

        let styled_sibling = &styled_body.children[1];
        // Sibling is not hovered/active; it is body's 2nd child, so `a:nth-child(2)` wins
        // (DROP-123); `::before` must not apply.
        assert_eq!(styled_sibling.style.color, Color::rgb(0x93, 0x33, 0xea));
        // The first link is the 1st child of `.card`: not :nth-child(2).
        assert_ne!(styled_link.style.color, Color::rgb(0x93, 0x33, 0xea));
    }

    #[test]
    fn test_flexbox_properties_border_radius_and_monospace_inheritance() {
        let mut doc = Document::new();
        let body = doc.create_element("body", Vec::new());
        let row = doc.create_element("div", attrs(&[("class", "row")]));
        let item1 = doc.create_element("div", attrs(&[("class", "item1")]));
        let item2 = doc.create_element("div", attrs(&[("class", "item2")]));
        let code = doc.create_element("code", Vec::new());
        let code_inner = doc.create_element("span", Vec::new());
        let code_txt = doc.create_text("let x = 1;");
        let code_override = doc.create_element("span", attrs(&[("style", "font-family: sans-serif;")]));
        let btn = doc.create_element("button", Vec::new());
        let img = doc.create_element("img", Vec::new());

        doc.append_child(doc.root(), body);
        doc.append_child(body, row);
        doc.append_child(row, item1);
        doc.append_child(row, item2);
        doc.append_child(body, code);
        doc.append_child(code, code_inner);
        doc.append_child(code_inner, code_txt);
        doc.append_child(code, code_override);
        doc.append_child(body, btn);
        doc.append_child(body, img);

        let css = css::parse(
            r#"
            .row {
                display: flex;
                flex-direction: column-reverse;
                flex-wrap: wrap;
                justify-content: space-between;
                align-items: center;
                gap: 1.5rem;
                border-radius: 12px;
            }
            .item1 {
                flex: 1;
            }
            .item2 {
                flex: 2 0 120px;
                font-family: "JetBrains Mono", Consolas, monospace;
            }
            "#,
        );

        let tree = style_tree(&doc, &css).unwrap();
        let styled_body = &tree.children[0];
        let styled_row = &styled_body.children[0];
        assert_eq!(styled_row.style.display, Display::Flex);
        assert_eq!(styled_row.style.flex_direction, FlexDirection::ColumnReverse);
        assert_eq!(styled_row.style.flex_wrap, FlexWrap::Wrap);
        assert_eq!(styled_row.style.justify_content, JustifyContent::SpaceBetween);
        assert_eq!(styled_row.style.align_items, AlignItems::Center);
        assert_eq!(styled_row.style.gap, 24.0);
        assert_eq!(styled_row.style.border_radius, 12.0);

        let styled_item1 = &styled_row.children[0];
        assert_eq!(styled_item1.style.flex_grow, 1.0);
        assert_eq!(styled_item1.style.flex_shrink, 1.0);
        assert_eq!(styled_item1.style.flex_basis, FlexBasis::Percent(0.0));

        let styled_item2 = &styled_row.children[1];
        assert_eq!(styled_item2.style.flex_grow, 2.0);
        assert_eq!(styled_item2.style.flex_shrink, 0.0);
        assert_eq!(styled_item2.style.flex_basis, FlexBasis::Px(120.0));
        assert!(styled_item2.style.monospace);

        // UA monospace inheritance on <code> -> <span> -> #text, and sans-serif override
        let styled_code = &styled_body.children[1];
        assert!(styled_code.style.monospace);
        let styled_code_inner = &styled_code.children[0];
        assert!(styled_code_inner.style.monospace);
        let styled_code_txt = &styled_code_inner.children[0];
        assert!(styled_code_txt.style.monospace);
        let styled_code_override = &styled_code.children[1];
        assert!(!styled_code_override.style.monospace);

        // UA defaults for <button> and <img>
        let styled_btn = &styled_body.children[2];
        assert_eq!(styled_btn.style.display, Display::InlineBlock);
        assert_eq!(styled_btn.style.padding.top, 6.0);
        assert_eq!(styled_btn.style.padding.left, 12.0);
        assert_eq!(styled_btn.style.border.top, 1.0);
        assert_eq!(styled_btn.style.border_radius, 6.0);
        assert_eq!(
            styled_btn.style.background_color,
            Some(Color::rgb(0xf8, 0xfa, 0xfc))
        );

        let styled_img = &styled_body.children[3];
        assert_eq!(styled_img.style.display, Display::InlineBlock);
    }

    #[test]
    fn test_ua_form_control_defaults_and_focus_pseudo() {
        let mut doc = Document::new();
        let body = doc.create_element("body", Vec::new());
        let input = doc.create_element(
            "input",
            attrs(&[("type", "text"), ("name", "q"), ("placeholder", "Поиск")]),
        );
        let area = doc.create_element("textarea", Vec::new());
        let select = doc.create_element("select", Vec::new());
        let option = doc.create_element("option", Vec::new());
        let button = doc.create_element("button", Vec::new());
        doc.append_child(doc.root(), body);
        doc.append_child(body, input);
        doc.append_child(body, area);
        doc.append_child(body, select);
        doc.append_child(select, option);
        doc.append_child(body, button);

        let css = css::parse("input:focus { border-color: #2563eb; }");
        assert!(stylesheet_uses_interaction(&css));

        let ctx = StyleContext {
            focused_node: Some(input),
            ..StyleContext::default()
        };
        let tree = style_tree_with_context(&doc, &css, &ctx).unwrap();
        let styled_body = &tree.children[0];
        let styled_input = &styled_body.children[0];
        assert_eq!(styled_input.style.display, Display::InlineBlock);
        assert_eq!(styled_input.style.font_size, 14.0);
        assert_eq!(styled_input.style.border_color, Color::rgb(0x25, 0x63, 0xeb));

        let unstyled_input = &style_tree_with_context(&doc, &css, &StyleContext::default())
            .unwrap()
            .children[0]
            .children[0];
        assert_ne!(
            unstyled_input.style.border_color,
            Color::rgb(0x25, 0x63, 0xeb)
        );

        let styled_area = &styled_body.children[1];
        assert_eq!(styled_area.style.width, LengthPct::Px(240.0));
        assert_eq!(styled_area.style.height, LengthPct::Px(64.0));
        assert_eq!(styled_area.style.padding.left, 8.0);

        let styled_select = &styled_body.children[2];
        assert_eq!(styled_select.style.display, Display::InlineBlock);
        assert_eq!(styled_select.style.padding.top, 5.0);

        let styled_button = &styled_body.children[3];
        assert_eq!(styled_button.style.display, Display::InlineBlock);
        assert_eq!(styled_button.style.border_radius, 6.0);
    }
}

#[cfg(test)]
mod index_equivalence_tests {
    use super::*;
    use droplet_dom::html;

    /// Deterministic xorshift64 so failures are reproducible.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
            items[(self.next() % items.len() as u64) as usize]
        }
        fn chance(&mut self, percent: u64) -> bool {
            self.next() % 100 < percent
        }
    }

    const TAGS: &[&str] = &["div", "span", "p", "a", "ul", "li", "section", "em"];
    const CLASSES: &[&str] = &["a", "b", "nav", "кнопка", "меню", "x-y", "item"];
    const IDS: &[&str] = &["main", "top", "заголовок", "z"];

    fn random_html(rng: &mut Rng, nodes: usize) -> String {
        let mut out = String::from("<!DOCTYPE html><html><body>");
        let mut open: Vec<&str> = Vec::new();
        for _ in 0..nodes {
            if !open.is_empty() && rng.chance(35) {
                let tag = open.pop().unwrap();
                out.push_str(&format!("</{tag}>"));
                continue;
            }
            let tag = rng.pick(TAGS);
            out.push('<');
            out.push_str(tag);
            if rng.chance(60) {
                let mut classes = vec![rng.pick(CLASSES)];
                if rng.chance(40) {
                    classes.push(rng.pick(CLASSES));
                }
                out.push_str(&format!(" class=\"{}\"", classes.join(" ")));
            }
            if rng.chance(15) {
                out.push_str(&format!(" id=\"{}\"", rng.pick(IDS)));
            }
            out.push('>');
            out.push_str("текст");
            open.push(tag);
        }
        while let Some(tag) = open.pop() {
            out.push_str(&format!("</{tag}>"));
        }
        out.push_str("</body></html>");
        out
    }

    fn random_compound(rng: &mut Rng) -> String {
        let mut s = String::new();
        match rng.next() % 5 {
            0 => s.push('*'),
            1 => s.push_str(&rng.pick(TAGS).to_ascii_uppercase()),
            2 => s.push_str(rng.pick(TAGS)),
            _ => {}
        }
        if rng.chance(20) {
            s.push('#');
            s.push_str(rng.pick(IDS));
        }
        let classes = rng.next() % 3;
        for _ in 0..classes {
            s.push('.');
            s.push_str(rng.pick(CLASSES));
        }
        if rng.chance(25) {
            s.push_str(rng.pick(&[
                ":hover",
                ":active",
                ":focus",
                ":visited",
                ":first-child",
                ":last-child",
                ":nth-child(odd)",
                ":nth-of-type(2n)",
                ":only-child",
                ":not(.a)",
                ":is(.меню, #top)",
                ":where(p, .b)",
                "[class]",
                "[class~=кнопка]",
                "[id^=з]",
                "[class*=-y i]",
                ":root",
                ":is(p > em, [id])",
                ":where(:not(.a), .b)",
                ":is(:visited, a)",
            ]));
        }
        if s.is_empty() {
            s.push_str(rng.pick(TAGS));
        }
        s
    }

    fn random_css(rng: &mut Rng, rules: usize) -> String {
        let mut out = String::new();
        for i in 0..rules {
            let selectors = 1 + rng.next() % 2;
            let mut list = Vec::new();
            for _ in 0..selectors {
                let parts = 1 + rng.next() % 3;
                let mut sel = random_compound(rng);
                for _ in 1..parts {
                    let comb = rng.pick(&[" > ", " ", " + ", " ~ "]);
                    sel = format!("{}{comb}{sel}", random_compound(rng));
                }
                list.push(sel);
            }
            out.push_str(&format!("{} {{ margin-left: {i}px }}\n", list.join(", ")));
        }
        out
    }

    fn assert_index_matches_bruteforce(seed: u64) {
        let mut rng = Rng(seed);
        let doc = html::parse(&random_html(&mut rng, 120));
        let sheet = css::parse(&random_css(&mut rng, 80));
        assert!(!sheet.rules.is_empty(), "seed {seed}: generated CSS parsed to nothing");

        let elements: Vec<NodeId> = (0..doc.nodes.len())
            .filter(|&id| doc.element(id).is_some())
            .collect();
        let ctx = StyleContext {
            hovered_node: elements.get(elements.len() / 2).copied(),
            active_node: elements.get(elements.len() / 3).copied(),
            focused_node: elements.last().copied(),
            ..StyleContext::default()
        };
        let cascade = CascadeContext {
            doc: &doc,
            ua_css: ua_stylesheet_ref(),
            ua_index: ua_index_ref(),
            author_css: &sheet,
            author_index: RuleIndex::build(&sheet),
            candidates: RefCell::new(Vec::new()),
            bloom: RefCell::new(AncestorBloom::new()),
            siblings: RefCell::new(SiblingIndex::default()),
            style_ctx: &ctx,
            hovered_ancestors: collect_ancestors(&doc, ctx.hovered_node),
            active_ancestors: collect_ancestors(&doc, ctx.active_node),
        };

        let mut total_matches = 0;
        for &id in &elements {
            let elem = doc.element(id).unwrap();
            // Recreate the Bloom state the depth-first walk would have for this node.
            let ancestors: Vec<&ElementData> = collect_ancestors(&doc, doc.get(id).and_then(|n| n.parent))
                .into_iter()
                .filter_map(|a| doc.element(a))
                .collect();
            for a in &ancestors {
                cascade.bloom.borrow_mut().push_element(a);
            }
            for (sheet, index) in [
                (cascade.author_css, &cascade.author_index),
                (cascade.ua_css, cascade.ua_index),
            ] {
                let fast = matching_rules(&cascade, id, elem, sheet, index);
                let slow = matching_rules_all(&cascade, id, sheet);
                let key = |v: &[(Specificity, usize, &[Declaration])]| {
                    v.iter().map(|(s, i, _)| (*s, *i)).collect::<Vec<_>>()
                };
                assert_eq!(key(&fast), key(&slow), "seed {seed}, node {id} <{}>", elem.tag_name);
                total_matches += fast.len();
            }
            for a in &ancestors {
                cascade.bloom.borrow_mut().pop_element(a);
            }
        }
        assert!(total_matches > 0, "seed {seed}: test generated no matches at all");
    }

    /// Which of the elements with an `id` match `selector`, in document order.
    fn matching_ids(html_src: &str, selector: &str) -> Vec<String> {
        let doc = html::parse(html_src);
        let sheet = css::parse(&format!("{selector} {{ color: red }}"));
        assert_eq!(sheet.rules.len(), 1, "{selector} did not parse");
        assert_eq!(sheet.rules[0].selectors.len(), 1, "{selector} did not parse");
        let ctx = StyleContext::default();
        let cascade = CascadeContext {
            doc: &doc,
            ua_css: ua_stylesheet_ref(),
            ua_index: ua_index_ref(),
            author_css: &sheet,
            author_index: RuleIndex::build(&sheet),
            candidates: RefCell::new(Vec::new()),
            bloom: RefCell::new(AncestorBloom::new()),
            siblings: RefCell::new(SiblingIndex::default()),
            style_ctx: &ctx,
            hovered_ancestors: Vec::new(),
            active_ancestors: Vec::new(),
        };
        (0..doc.nodes.len())
            .filter(|&id| matches_complex(&cascade, id, &sheet.rules[0].selectors[0]))
            .filter_map(|id| doc.element(id).and_then(|e| e.id()).map(str::to_string))
            .collect()
    }

    #[test]
    fn structural_attribute_logical_and_sibling_selectors_match() {
        let page = r#"<html id="html"><body id="body">
            <ul id="ul">
              <li id="l1" class="пункт">один</li>
              <li id="l2" data-x="1" lang="ru-RU">two</li>
              <p id="p1" title="Привет мир"></p>
              <li id="l3" class="пункт важный">три</li>
              <li id="l4" class="skip" data-тип="Кнопка">skip</li>
            </ul>
            <div id="d1"><span id="s1">only</span></div>
            <h2 id="h">t</h2><p id="p2">a</p><p id="p3">b</p>
            </body></html>"#;
        let ids = |sel| matching_ids(page, sel);
        let table: &[(&str, &[&str])] = &[
            ("li:nth-child(odd)", &["l1", "l4"]), // <p> is child 3
            ("li:nth-child(even)", &["l2", "l3"]),
            ("#ul > :nth-child(3)", &["p1"]),
            ("li:nth-child(-n+2)", &["l1", "l2"]),
            ("#ul > :nth-last-child(1)", &["l4"]),
            ("li:nth-of-type(3)", &["l3"]),
            ("li:nth-last-of-type(odd)", &["l2", "l4"]),
            ("li:first-child", &["l1"]),
            ("li:last-child", &["l4"]),
            ("#ul p:first-of-type", &["p1"]),
            ("#ul > :only-of-type", &["p1"]),
            (":only-child", &["html", "body", "s1"]), // no <head> here
            ("#ul > p:empty", &["p1"]),
            (":root", &["html"]),
            ("li:nth-child(2 of .пункт)", &["l3"]),
            ("li:nth-last-child(1 of :not(.skip))", &["l3"]),
            ("[data-x]", &["l2"]),
            ("li[data-x='1']", &["l2"]),
            ("[lang|=ru]", &["l2"]),
            ("[class~=важный]", &["l3"]),
            ("[class^=пун]", &["l1", "l3"]),
            ("[class$=ный]", &["l3"]),
            ("[title*='т м']", &["p1"]),
            ("[data-тип=кнопка]", &[]),
            ("[data-тип=Кнопка]", &["l4"]),
            ("[data-x=1 i]", &["l2"]),
            ("li:not(.skip)", &["l1", "l2", "l3"]),
            ("li:not(.skip, [data-x])", &["l1", "l3"]),
            ("li:not(:first-child):not(:last-child)", &["l2", "l3"]),
            (":is(#l1, .skip)", &["l1", "l4"]),
            (":where(ul, div) > :is(span, .важный)", &["l3", "s1"]),
            (".пункт.важный", &["l3"]),
            ("h2 + p", &["p2"]),
            ("h2 ~ p", &["p2", "p3"]),
            ("#l1 + li", &["l2"]),
            ("#l2 + li", &[]),
            ("#l2 ~ li", &["l3", "l4"]),
            ("li + p ~ li", &["l3", "l4"]),
            ("body > ul ~ div span", &["s1"]),
            ("#ul > li:first-child + li + p", &["p1"]),
            ("p:not(#ul p)", &["p2", "p3"]),
        ];
        for &(sel, expected) in table {
            assert_eq!(ids(sel), expected, "{sel}");
        }
    }

    #[test]
    fn rule_index_gives_identical_matches_to_testing_every_rule() {
        for seed in 1..=40u64 {
            assert_index_matches_bruteforce(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        }
    }

    #[test]
    fn full_style_tree_unchanged_on_cyrillic_page() {
        let doc = html::parse(
            "<html><body><div id=\"заголовок\" class=\"меню a\"><p class=\"кнопка\">Привет</p>\
             <span>мир</span></div></body></html>",
        );
        let sheet = css::parse(
            "#заголовок { color: red } .меню p { margin-left: 5px } P.кнопка { padding-top: 3px }\
             div > span { color: blue } * { margin-top: 1px }",
        );
        let styled = style_tree(&doc, &sheet).expect("styled tree");
        let mut stack = vec![&styled];
        let mut seen_p = false;
        while let Some(node) = stack.pop() {
            if let Some(elem) = doc.element(node.node_id) {
                if elem.tag_name == "p" {
                    seen_p = true;
                    assert_eq!(node.style.margin.left, 5.0);
                    assert_eq!(node.style.padding.top, 3.0);
                    assert_eq!(node.style.margin.top, 1.0);
                    assert_eq!(node.style.color, Color::rgb(255, 0, 0));
                }
            }
            stack.extend(node.children.iter());
        }
        assert!(seen_p);
    }
}
