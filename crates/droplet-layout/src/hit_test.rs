use droplet_dom::NodeId;

use crate::box_model::LayoutBox;

pub fn hit_test(root: &LayoutBox, x: f32, y: f32) -> Option<&LayoutBox> {
    for child in root.children.iter().rev() {
        if let Some(hit) = hit_test(child, x, y) {
            return Some(hit);
        }
    }
    if root.dimensions.border_box().contains(x, y) {
        Some(root)
    } else {
        None
    }
}

pub fn hit_test_href(root: &LayoutBox, x: f32, y: f32) -> Option<&str> {
    hit_test_attr(root, x, y, pick_href).1
}

pub fn hit_test_node(root: &LayoutBox, x: f32, y: f32) -> Option<NodeId> {
    hit_test_attr(root, x, y, pick_node).1
}

fn pick_href(node: &LayoutBox) -> Option<&str> {
    node.href.as_deref()
}

fn pick_node(node: &LayoutBox) -> Option<NodeId> {
    node.node_id
}

fn hit_test_attr<'a, T>(
    node: &'a LayoutBox,
    x: f32,
    y: f32,
    pick: fn(&'a LayoutBox) -> Option<T>,
) -> (bool, Option<T>) {
    for child in node.children.iter().rev() {
        let (hit, val) = hit_test_attr(child, x, y, pick);
        if hit {
            return (true, val.or_else(|| pick(node)));
        }
    }
    if node.dimensions.border_box().contains(x, y) {
        (true, pick(node))
    } else {
        (false, None)
    }
}
