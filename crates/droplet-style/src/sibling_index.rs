//! Per-element sibling positions for structural selectors (DROP-123).
//!
//! `:nth-child`, `:last-of-type`, `+` and friends need an element's position among its
//! siblings. Scanning the parent's child list for every element and rule would be O(n²) on
//! long lists, so the first query for any child of a parent fills the positions of all that
//! parent's children in one pass. Only sibling groups that a structural selector actually
//! reaches are computed; pages without such selectors never allocate the table.

use droplet_dom::{Document, NodeId, NodeKind};

/// 1-based positions among element siblings.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct SiblingPos {
    /// Index of the node in its parent's `children`.
    pub child_pos: u32,
    pub elem_index: u32,
    /// Element siblings including this one; `0` = not computed yet (or not an element child).
    pub elem_count: u32,
    /// Position among siblings with the same tag name.
    pub type_index: u32,
    pub type_count: u32,
}

/// Lazily filled `NodeId -> SiblingPos` table, valid for one style pass.
#[derive(Debug, Default)]
pub(crate) struct SiblingIndex {
    table: Vec<SiblingPos>,
    /// (tag, count) scratch for the parent being filled.
    type_counts: Vec<(String, u32)>,
}

impl SiblingIndex {
    /// Position of `id` among its siblings; all zero for non-elements and parentless nodes.
    pub(crate) fn get(&mut self, doc: &Document, id: NodeId) -> SiblingPos {
        if self.table.len() != doc.nodes.len() {
            self.table = vec![SiblingPos::default(); doc.nodes.len()];
        }
        match self.table.get(id) {
            Some(pos) if pos.elem_count != 0 => return *pos,
            Some(_) => {}
            None => return SiblingPos::default(),
        }
        if doc.element(id).is_none() {
            return SiblingPos::default();
        }
        if let Some(parent) = doc.get(id).and_then(|n| n.parent) {
            self.fill_children_of(doc, parent);
        }
        self.table.get(id).copied().unwrap_or_default()
    }

    fn fill_children_of(&mut self, doc: &Document, parent: NodeId) {
        let Some(node) = doc.get(parent) else {
            return;
        };
        self.type_counts.clear();
        let mut elem_count = 0u32;
        for (pos, &child) in node.children.iter().enumerate() {
            let Some(NodeKind::Element(elem)) = doc.get(child).map(|n| &n.kind) else {
                continue;
            };
            elem_count += 1;
            let tag = elem.tag_name.as_str();
            let type_index = match self
                .type_counts
                .iter_mut()
                .find(|(t, _)| t.eq_ignore_ascii_case(tag))
            {
                Some((_, count)) => {
                    *count += 1;
                    *count
                }
                None => {
                    self.type_counts.push((tag.to_string(), 1));
                    1
                }
            };
            if let Some(entry) = self.table.get_mut(child) {
                *entry = SiblingPos {
                    child_pos: pos as u32,
                    elem_index: elem_count,
                    elem_count: 0,
                    type_index,
                    type_count: 0,
                };
            }
        }
        for &child in &node.children {
            let Some(NodeKind::Element(elem)) = doc.get(child).map(|n| &n.kind) else {
                continue;
            };
            let tag = elem.tag_name.as_str();
            let type_count = self
                .type_counts
                .iter()
                .find(|(t, _)| t.eq_ignore_ascii_case(tag))
                .map_or(1, |&(_, c)| c);
            if let Some(entry) = self.table.get_mut(child) {
                entry.elem_count = elem_count;
                entry.type_count = type_count;
            }
        }
    }

    /// The element sibling just before `id`, if any.
    pub(crate) fn prev_element_sibling(&mut self, doc: &Document, id: NodeId) -> Option<NodeId> {
        let parent = doc.get(id)?.parent?;
        let pos = self.get(doc, id).child_pos as usize;
        doc.get(parent)?
            .children
            .get(..pos)?
            .iter()
            .rev()
            .copied()
            .find(|&s| doc.element(s).is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use droplet_dom::html;

    #[test]
    fn positions_skip_text_and_count_types() {
        let doc = html::parse(
            "<html><body><ul> <li>один</li> <p>x</p> <li>два</li><li>три</li> </ul></body></html>",
        );
        let mut index = SiblingIndex::default();
        let ul = (0..doc.nodes.len())
            .find(|&id| doc.element(id).is_some_and(|e| e.tag_name == "ul"))
            .unwrap();
        let kids: Vec<NodeId> = doc.nodes[ul]
            .children
            .iter()
            .copied()
            .filter(|&c| doc.element(c).is_some())
            .collect();
        assert_eq!(kids.len(), 4);
        let pos: Vec<(u32, u32, u32, u32)> = kids
            .iter()
            .rev() // query the last child first: the whole group must be filled correctly
            .map(|&k| {
                let p = index.get(&doc, k);
                (p.elem_index, p.elem_count, p.type_index, p.type_count)
            })
            .collect();
        assert_eq!(pos, vec![(4, 4, 3, 3), (3, 4, 2, 3), (2, 4, 1, 1), (1, 4, 1, 3)]);
        assert_eq!(index.prev_element_sibling(&doc, kids[2]), Some(kids[1]));
        assert_eq!(index.prev_element_sibling(&doc, kids[0]), None);
        // Text nodes and the document have no element position.
        let text = doc.nodes[ul].children[0];
        assert!(doc.element(text).is_none());
        assert_eq!(index.get(&doc, text), SiblingPos::default());
        assert_eq!(index.get(&doc, doc.root()), SiblingPos::default());
        // Only `ul`'s children were computed.
        let computed = index.table.iter().filter(|p| p.elem_count != 0).count();
        assert_eq!(computed, 4);
    }
}
