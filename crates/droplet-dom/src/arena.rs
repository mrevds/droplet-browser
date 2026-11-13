use std::fmt::Write;

pub type NodeId = usize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElementData {
    pub tag_name: String,
    pub attributes: Vec<(String, String)>,
}

impl ElementData {
    pub fn id(&self) -> Option<&str> {
        self.attr("id")
    }

    pub fn classes(&self) -> Vec<&str> {
        self.attr("class")
            .map(|c| c.split_whitespace().collect())
            .unwrap_or_default()
    }

    pub fn has_class(&self, class: &str) -> bool {
        self.attr("class")
            .is_some_and(|c| c.split_whitespace().any(|item| item == class))
    }

    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeKind {
    Document,
    Element(ElementData),
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub id: NodeId,
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    pub kind: NodeKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    pub nodes: Vec<Node>,
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

impl Document {
    pub fn new() -> Self {
        Self {
            nodes: vec![Node {
                id: 0,
                parent: None,
                children: Vec::new(),
                kind: NodeKind::Document,
            }],
        }
    }

    pub fn root(&self) -> NodeId {
        0
    }

    pub fn get(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id)
    }

    pub fn create_element<I>(&mut self, tag_name: &str, attributes: I) -> NodeId
    where
        I: IntoIterator<Item = (String, String)>,
    {
        let mut attrs: Vec<(String, String)> = Vec::new();
        for (key, value) in attributes {
            if !attrs.iter().any(|(existing, _)| existing == &key) {
                attrs.push((key, value));
            }
        }
        let id = self.nodes.len();
        self.nodes.push(Node {
            id,
            parent: None,
            children: Vec::new(),
            kind: NodeKind::Element(ElementData {
                tag_name: tag_name.to_ascii_lowercase(),
                attributes: attrs,
            }),
        });
        id
    }

    pub fn create_text(&mut self, text: &str) -> NodeId {
        let id = self.nodes.len();
        self.nodes.push(Node {
            id,
            parent: None,
            children: Vec::new(),
            kind: NodeKind::Text(text.to_string()),
        });
        id
    }

    pub fn append_child(&mut self, parent: NodeId, child: NodeId) {
        if parent >= self.nodes.len()
            || child >= self.nodes.len()
            || parent == child
            || child == 0
        {
            return;
        }
        if let Some(old_parent) = self.nodes[child].parent {
            if let Some(p_node) = self.nodes.get_mut(old_parent) {
                p_node.children.retain(|&c| c != child);
            }
        }
        self.nodes[child].parent = Some(parent);
        self.nodes[parent].children.push(child);
    }

    pub fn element(&self, id: NodeId) -> Option<&ElementData> {
        match self.nodes.get(id) {
            Some(Node {
                kind: NodeKind::Element(data),
                ..
            }) => Some(data),
            _ => None,
        }
    }

    pub fn text_content(&self, id: NodeId) -> String {
        if id >= self.nodes.len() {
            return String::new();
        }
        let mut out = String::new();
        let mut stack = vec![id];
        while let Some(curr) = stack.pop() {
            let Some(node) = self.nodes.get(curr) else {
                continue;
            };
            match &node.kind {
                NodeKind::Text(text) => out.push_str(text),
                NodeKind::Document | NodeKind::Element(_) => {
                    for &child in node.children.iter().rev() {
                        stack.push(child);
                    }
                }
            }
        }
        out
    }

    pub fn collect_style_text(&self) -> String {
        let mut styles = Vec::new();
        let mut stack = vec![self.root()];
        while let Some(curr) = stack.pop() {
            let Some(node) = self.nodes.get(curr) else {
                continue;
            };
            if let NodeKind::Element(data) = &node.kind {
                if data.tag_name == "style" {
                    let css = self.text_content(curr);
                    if !css.trim().is_empty() {
                        styles.push(css);
                    }
                    continue;
                }
            }
            for &child in node.children.iter().rev() {
                stack.push(child);
            }
        }
        styles.join("\n")
    }

    pub fn collect_stylesheet_links(&self) -> Vec<String> {
        let mut links = Vec::new();
        let mut stack = vec![self.root()];
        while let Some(curr) = stack.pop() {
            let Some(node) = self.nodes.get(curr) else {
                continue;
            };
            if let NodeKind::Element(data) = &node.kind {
                if data.tag_name.eq_ignore_ascii_case("link") {
                    let is_stylesheet = data.attr("rel").is_some_and(|rel| {
                        rel.split_ascii_whitespace()
                            .any(|t| t.eq_ignore_ascii_case("stylesheet"))
                    });
                    if is_stylesheet {
                        if let Some(href) = data.attr("href") {
                            let trimmed = href.trim();
                            if !trimmed.is_empty() {
                                links.push(trimmed.to_string());
                            }
                        }
                    }
                }
            }
            for &child in node.children.iter().rev() {
                stack.push(child);
            }
        }
        links
    }

    pub fn find_title(&self) -> Option<String> {
        let title_id = self.find_first_tag("title")?;
        let text = self.text_content(title_id);
        let trimmed = text.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    }

    pub fn find_first_tag(&self, tag: &str) -> Option<NodeId> {
        let mut stack = vec![self.root()];
        while let Some(curr) = stack.pop() {
            let Some(node) = self.nodes.get(curr) else {
                continue;
            };
            if let NodeKind::Element(data) = &node.kind {
                if data.tag_name.eq_ignore_ascii_case(tag) {
                    return Some(curr);
                }
            }
            for &child in node.children.iter().rev() {
                stack.push(child);
            }
        }
        None
    }

    pub fn dump_tree(&self) -> String {
        let mut out = String::new();
        let mut stack = vec![(self.root(), 0usize)];
        while let Some((curr, depth)) = stack.pop() {
            let Some(node) = self.nodes.get(curr) else {
                continue;
            };
            let indent = "  ".repeat(depth);
            match &node.kind {
                NodeKind::Document => {
                    let _ = writeln!(out, "{indent}Document");
                }
                NodeKind::Element(data) => {
                    let _ = write!(out, "{indent}<{}", data.tag_name);
                    if !data.attributes.is_empty() {
                        let mut sorted_attrs: Vec<&(String, String)> =
                            data.attributes.iter().collect();
                        sorted_attrs.sort_by(|a, b| a.0.cmp(&b.0));
                        for (key, val) in sorted_attrs {
                            let _ = write!(out, " {key}=\"{val}\"");
                        }
                    }
                    let _ = writeln!(out, ">");
                }
                NodeKind::Text(text) => {
                    let _ = writeln!(out, "{indent}#text {text:?}");
                }
            }
            for &child in node.children.iter().rev() {
                stack.push((child, depth + 1));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    #[test]
    fn arena_dom_basics_and_element_helpers() {
        let mut doc = Document::new();
        assert_eq!(doc.root(), 0);
        assert!(matches!(doc.get(0).unwrap().kind, NodeKind::Document));

        let mut attrs = HashMap::new();
        attrs.insert("id".into(), "hero".into());
        attrs.insert("class".into(), "card  primary  featured".into());
        attrs.insert("data-x".into(), "42".into());

        let div = doc.create_element("DIV", attrs);
        let text1 = doc.create_text("Привет, ");
        let span = doc.create_element("span", HashMap::new());
        let text2 = doc.create_text("Droplet!");

        doc.append_child(doc.root(), div);
        doc.append_child(div, text1);
        doc.append_child(div, span);
        doc.append_child(span, text2);

        let elem = doc.element(div).unwrap();
        assert_eq!(elem.tag_name, "div");
        assert_eq!(elem.id(), Some("hero"));
        assert_eq!(elem.classes(), vec!["card", "primary", "featured"]);
        assert!(elem.has_class("primary"));
        assert!(!elem.has_class("prim"));
        assert_eq!(elem.attr("data-x"), Some("42"));
        assert_eq!(elem.attr("missing"), None);

        assert_eq!(doc.text_content(div), "Привет, Droplet!");
        assert_eq!(doc.find_first_tag("SPAN"), Some(span));
        assert_eq!(doc.get(span).unwrap().parent, Some(div));
    }

    #[test]
    fn collect_style_title_and_dump_tree() {
        let mut doc = Document::default();
        let head = doc.create_element("head", HashMap::new());
        let title = doc.create_element("title", HashMap::new());
        let title_txt = doc.create_text("  Droplet Browser  ");
        let style1 = doc.create_element("style", HashMap::new());
        let css1 = doc.create_text("body { color: #111; }");
        let style2 = doc.create_element("style", HashMap::new());
        let css2 = doc.create_text("h1 { color: #00f; }");

        doc.append_child(doc.root(), head);
        doc.append_child(head, title);
        doc.append_child(title, title_txt);
        doc.append_child(head, style1);
        doc.append_child(style1, css1);
        doc.append_child(head, style2);
        doc.append_child(style2, css2);

        assert_eq!(doc.find_title().as_deref(), Some("Droplet Browser"));
        assert_eq!(
            doc.collect_style_text(),
            "body { color: #111; }\nh1 { color: #00f; }"
        );
        let dump = doc.dump_tree();
        assert!(dump.contains("Document\n"));
        assert!(dump.contains("  <head>\n"));
        assert!(dump.contains("    <title>\n"));
    }

    #[test]
    fn deep_tree_does_not_overflow_stack() {
        let mut doc = Document::new();
        let mut curr = doc.root();
        for _ in 0..15_000 {
            let next = doc.create_element("div", HashMap::new());
            doc.append_child(curr, next);
            curr = next;
        }
        let leaf = doc.create_text("deep");
        doc.append_child(curr, leaf);

        assert_eq!(doc.text_content(doc.root()), "deep");
        let dump = doc.dump_tree();
        assert!(dump.contains("#text \"deep\""));
    }

    #[test]
    fn collect_stylesheet_links_filters_and_preserves_document_order() {
        let mut doc = Document::new();
        let html = doc.create_element("html", Vec::new());
        let head = doc.create_element("head", Vec::new());
        let body = doc.create_element("body", Vec::new());
        doc.append_child(doc.root(), html);
        doc.append_child(html, head);
        doc.append_child(html, body);

        let icon = doc.create_element(
            "link",
            vec![
                ("rel".into(), "icon".into()),
                ("href".into(), "/favicon.ico".into()),
            ],
        );
        let sheet1 = doc.create_element(
            "link",
            vec![
                ("rel".into(), "stylesheet".into()),
                ("href".into(), "/base.css".into()),
            ],
        );
        let preload_sheet = doc.create_element(
            "link",
            vec![
                ("rel".into(), "preload StyleSheet".into()),
                ("href".into(), "https://cdn.example.com/theme.css".into()),
            ],
        );
        let missing_href = doc.create_element("link", vec![("rel".into(), "stylesheet".into())]);
        let blank_href = doc.create_element(
            "link",
            vec![
                ("rel".into(), "stylesheet".into()),
                ("href".into(), "   ".into()),
            ],
        );
        let partial_token = doc.create_element(
            "link",
            vec![
                ("rel".into(), "my-stylesheet".into()),
                ("href".into(), "/ignored.css".into()),
            ],
        );
        let body_sheet = doc.create_element(
            "link",
            vec![
                ("rel".into(), "stylesheet".into()),
                ("href".into(), "components/card.css".into()),
            ],
        );

        for child in [
            icon,
            sheet1,
            preload_sheet,
            missing_href,
            blank_href,
            partial_token,
        ] {
            doc.append_child(head, child);
        }
        doc.append_child(body, body_sheet);

        assert_eq!(
            doc.collect_stylesheet_links(),
            vec![
                "/base.css",
                "https://cdn.example.com/theme.css",
                "components/card.css",
            ]
        );
    }

    #[test]
    fn create_element_deduplicates_and_zero_allocs_when_empty() {
        let mut doc = Document::new();
        let empty = doc.create_element("p", Vec::new());
        assert_eq!(doc.element(empty).unwrap().attributes.capacity(), 0);

        let dup = doc.create_element(
            "div",
            vec![
                ("z-index".into(), "10".into()),
                ("id".into(), "first".into()),
                ("id".into(), "second".into()),
                ("class".into(), "a".into()),
            ],
        );
        doc.append_child(doc.root(), dup);

        let elem = doc.element(dup).unwrap();
        assert_eq!(elem.id(), Some("first"));
        assert_eq!(
            elem.attributes,
            vec![
                ("z-index".into(), "10".into()),
                ("id".into(), "first".into()),
                ("class".into(), "a".into()),
            ]
        );
        assert_eq!(
            doc.dump_tree(),
            "Document\n  <div class=\"a\" id=\"first\" z-index=\"10\">\n"
        );
    }
}
