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

    pub fn append_child(&mut self, parent: NodeId, child: NodeId) -> bool {
        if parent >= self.nodes.len()
            || child >= self.nodes.len()
            || parent == child
            || child == 0
        {
            return false;
        }
        // Guard against ancestor cycles so parent chains remain acyclic.
        let mut anc = Some(parent);
        let mut steps = 0usize;
        while let Some(curr) = anc {
            if curr == child {
                return false;
            }
            steps += 1;
            if steps > self.nodes.len() {
                return false;
            }
            anc = self.nodes.get(curr).and_then(|n| n.parent);
        }
        if let Some(old_parent) = self.nodes[child].parent {
            if let Some(p_node) = self.nodes.get_mut(old_parent) {
                p_node.children.retain(|&c| c != child);
            }
        }
        self.nodes[child].parent = Some(parent);
        self.nodes[parent].children.push(child);
        true
    }

    pub fn remove_child(&mut self, parent: NodeId, child: NodeId) -> bool {
        if parent >= self.nodes.len()
            || child >= self.nodes.len()
            || self.nodes[child].parent != Some(parent)
        {
            return false;
        }
        self.nodes[parent].children.retain(|&c| c != child);
        self.nodes[child].parent = None;
        true
    }

    pub fn parent_node(&self, id: NodeId) -> Option<NodeId> {
        self.nodes.get(id).and_then(|n| n.parent)
    }

    pub fn document_body(&self) -> Option<NodeId> {
        self.find_first_tag("body")
    }

    pub fn set_document_title(&mut self, title: &str) {
        if let Some(title_id) = self.find_first_tag("title") {
            self.set_text(title_id, title);
            return;
        }
        let host = self
            .find_first_tag("head")
            .or_else(|| self.find_first_tag("html"))
            .unwrap_or_else(|| self.root());
        let title_id = self.create_element("title", Vec::new());
        self.set_text(title_id, title);
        let _ = self.append_child(host, title_id);
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

    pub fn element_mut(&mut self, id: NodeId) -> Option<&mut ElementData> {
        match self.nodes.get_mut(id) {
            Some(Node {
                kind: NodeKind::Element(data),
                ..
            }) => Some(data),
            _ => None,
        }
    }

    /// Sets (or inserts) an element attribute in place, preserving flat `Vec` attribute storage.
    pub fn set_attr(&mut self, id: NodeId, name: &str, value: &str) {
        let Some(elem) = self.element_mut(id) else {
            return;
        };
        if let Some((_, existing)) = elem.attributes.iter_mut().find(|(k, _)| k == name) {
            existing.clear();
            existing.push_str(value);
        } else {
            elem.attributes.push((name.to_string(), value.to_string()));
        }
    }

    /// Removes an element attribute if present.
    pub fn remove_attr(&mut self, id: NodeId, name: &str) {
        let Some(elem) = self.element_mut(id) else {
            return;
        };
        elem.attributes.retain(|(k, _)| k != name);
    }

    /// Replaces an element's text content in place. Existing direct text children are reused;
    /// additional children are detached first so textarea values never accumulate stale nodes.
    pub fn set_text(&mut self, id: NodeId, text: &str) {
        if self.element(id).is_none() {
            return;
        }
        let children = self.nodes[id].children.clone();
        let mut target: Option<NodeId> = None;
        for child in children {
            if target.is_none() && matches!(self.nodes.get(child), Some(Node { kind: NodeKind::Text(_), .. })) {
                target = Some(child);
            } else if let Some(node) = self.nodes.get_mut(child) {
                node.parent = None;
            }
        }
        self.nodes[id]
            .children
            .retain(|child| Some(*child) == target);
        match target {
            Some(text_id) => {
                if let Some(node) = self.nodes.get_mut(text_id) {
                    node.kind = NodeKind::Text(text.to_string());
                }
            }
            None => {
                let text_id = self.create_text(text);
                self.append_child(id, text_id);
            }
        }
    }

    pub fn text_content_of(&self, id: NodeId) -> String {
        self.text_content(id)
    }

    pub fn set_text_content_of(&mut self, id: NodeId, text: &str) {
        if let Some(node) = self.nodes.get_mut(id) {
            if let NodeKind::Text(existing) = &mut node.kind {
                existing.clear();
                existing.push_str(text);
                return;
            }
        }
        self.set_text(id, text);
    }

    pub fn serialize_inner_html(&self, id: NodeId) -> String {
        let Some(root_node) = self.nodes.get(id) else {
            return String::new();
        };
        if matches!(root_node.kind, NodeKind::Text(_)) {
            return String::new();
        }
        enum SerFrame {
            Enter { id: NodeId, raw_text: bool },
            Close(String),
        }
        let raw_parent = matches!(
            &root_node.kind,
            NodeKind::Element(e)
                if e.tag_name.eq_ignore_ascii_case("style")
                    || e.tag_name.eq_ignore_ascii_case("script")
        );
        let mut out = String::new();
        let mut stack = Vec::with_capacity(root_node.children.len());
        for &child in root_node.children.iter().rev() {
            stack.push(SerFrame::Enter {
                id: child,
                raw_text: raw_parent,
            });
        }
        while let Some(frame) = stack.pop() {
            match frame {
                SerFrame::Close(tag) => {
                    let _ = write!(out, "</{tag}>");
                }
                SerFrame::Enter { id: curr, raw_text } => {
                    let Some(node) = self.nodes.get(curr) else {
                        continue;
                    };
                    match &node.kind {
                        NodeKind::Document => {
                            for &child in node.children.iter().rev() {
                                stack.push(SerFrame::Enter {
                                    id: child,
                                    raw_text: false,
                                });
                            }
                        }
                        NodeKind::Text(text) => {
                            if raw_text {
                                out.push_str(text);
                            } else {
                                push_escaped_html_text(&mut out, text);
                            }
                        }
                        NodeKind::Element(data) => {
                            out.push('<');
                            out.push_str(&data.tag_name);
                            for (key, val) in &data.attributes {
                                out.push(' ');
                                out.push_str(key);
                                out.push_str("=\"");
                                push_escaped_attr_value(&mut out, val);
                                out.push('"');
                            }
                            out.push('>');
                            if crate::html::is_void_tag(&data.tag_name) {
                                continue;
                            }
                            let child_raw = data.tag_name.eq_ignore_ascii_case("style")
                                || data.tag_name.eq_ignore_ascii_case("script");
                            stack.push(SerFrame::Close(data.tag_name.clone()));
                            for &child in node.children.iter().rev() {
                                stack.push(SerFrame::Enter {
                                    id: child,
                                    raw_text: child_raw,
                                });
                            }
                        }
                    }
                }
            }
        }
        out
    }

    pub fn set_inner_html_fragment(&mut self, id: NodeId, html_src: &str) -> bool {
        if self.element(id).is_none() {
            return false;
        }
        let old_children = std::mem::take(&mut self.nodes[id].children);
        for child in old_children {
            if let Some(node) = self.nodes.get_mut(child) {
                node.parent = None;
            }
        }
        if html_src.is_empty() {
            return true;
        }

        let frag = crate::html::parse(html_src);
        let mut stack: Vec<(NodeId, NodeId)> = frag.nodes[frag.root()]
            .children
            .iter()
            .rev()
            .map(|&child| (child, id))
            .collect();
        while let Some((frag_id, dest_parent)) = stack.pop() {
            let Some(frag_node) = frag.get(frag_id) else {
                continue;
            };
            match &frag_node.kind {
                NodeKind::Document => {}
                NodeKind::Text(text) => {
                    let tid = self.create_text(text);
                    let _ = self.append_child(dest_parent, tid);
                }
                NodeKind::Element(data) => {
                    let eid = self.create_element(&data.tag_name, data.attributes.clone());
                    let _ = self.append_child(dest_parent, eid);
                    for &child in frag_node.children.iter().rev() {
                        stack.push((child, eid));
                    }
                }
            }
        }
        true
    }

    pub fn collect_script_sources(&self) -> Vec<(Option<String>, Option<String>)> {
        let mut scripts = Vec::new();
        let mut stack = vec![self.root()];
        while let Some(curr) = stack.pop() {
            let Some(node) = self.nodes.get(curr) else {
                continue;
            };
            if let NodeKind::Element(data) = &node.kind {
                if data.tag_name.eq_ignore_ascii_case("script") {
                    if is_js_script_type(data.attr("type")) {
                        let src = data
                            .attr("src")
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(ToString::to_string);
                        if let Some(src_val) = src {
                            scripts.push((Some(src_val), None));
                        } else {
                            let code = self.text_content(curr);
                            if !code.trim().is_empty() {
                                scripts.push((None, Some(code)));
                            }
                        }
                    }
                    continue;
                }
            }
            for &child in node.children.iter().rev() {
                stack.push(child);
            }
        }
        scripts
    }

    pub fn get_element_by_id(&self, id: &str) -> Option<NodeId> {
        crate::query::find_element_by_id(self, id)
    }

    pub fn query_selector(&self, selector: &str) -> Option<NodeId> {
        crate::query::query_first(self, selector)
    }

    pub fn query_selector_all(&self, selector: &str) -> Vec<NodeId> {
        crate::query::query_all(self, selector)
    }

    pub fn class_list_contains(&self, id: NodeId, class_name: &str) -> bool {
        let token = class_name.trim();
        if token.is_empty() || token.contains(char::is_whitespace) {
            return false;
        }
        self.element(id).is_some_and(|e| e.has_class(token))
    }

    pub fn class_list_add(&mut self, id: NodeId, class_name: &str) {
        let token = class_name.trim();
        if token.is_empty() || token.contains(char::is_whitespace) {
            return;
        }
        let Some(elem) = self.element(id) else {
            return;
        };
        if elem.has_class(token) {
            return;
        }
        let next = match elem.attr("class") {
            Some(existing) if !existing.trim().is_empty() => format!("{} {token}", existing.trim()),
            _ => token.to_string(),
        };
        self.set_attr(id, "class", &next);
    }

    pub fn class_list_remove(&mut self, id: NodeId, class_name: &str) {
        let token = class_name.trim();
        if token.is_empty() || token.contains(char::is_whitespace) {
            return;
        }
        let Some(elem) = self.element(id) else {
            return;
        };
        let Some(existing) = elem.attr("class") else {
            return;
        };
        let remaining: Vec<&str> = existing
            .split_whitespace()
            .filter(|&item| item != token)
            .collect();
        if remaining.is_empty() {
            self.remove_attr(id, "class");
        } else {
            let joined = remaining.join(" ");
            self.set_attr(id, "class", &joined);
        }
    }

    pub fn class_list_toggle(&mut self, id: NodeId, class_name: &str) -> bool {
        let token = class_name.trim();
        if token.is_empty() || token.contains(char::is_whitespace) {
            return false;
        }
        if self.class_list_contains(id, token) {
            self.class_list_remove(id, token);
            false
        } else {
            self.class_list_add(id, token);
            self.element(id).is_some_and(|e| e.has_class(token))
        }
    }

    pub fn style_prop_get(&self, id: NodeId, prop: &str) -> String {
        crate::query::style_prop_get(self, id, prop)
    }

    pub fn style_prop_set(&mut self, id: NodeId, prop: &str, value: &str) {
        crate::query::style_prop_set(self, id, prop, value);
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

fn push_escaped_html_text(out: &mut String, text: &str) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
}

fn push_escaped_attr_value(out: &mut String, value: &str) {
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
}

fn is_js_script_type(type_attr: Option<&str>) -> bool {
    let Some(raw) = type_attr else {
        return true;
    };
    let mime = raw.split(';').next().unwrap_or("").trim();
    if mime.is_empty() {
        return true;
    }
    mime.eq_ignore_ascii_case("text/javascript")
        || mime.eq_ignore_ascii_case("application/javascript")
        || mime.eq_ignore_ascii_case("text/ecmascript")
        || mime.eq_ignore_ascii_case("application/ecmascript")
        || mime.eq_ignore_ascii_case("application/x-javascript")
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
    fn mutation_helpers_set_remove_attrs_and_replace_text_utf8_safely() {
        let mut doc = Document::new();
        let input = doc.create_element(
            "input",
            vec![("type".into(), "text".into()), ("value".into(), "old".into())],
        );
        doc.append_child(doc.root(), input);

        doc.set_attr(input, "value", "Привет, мир");
        assert_eq!(doc.element(input).unwrap().attr("value"), Some("Привет, мир"));
        doc.set_attr(input, "value", "новое значение");
        assert_eq!(
            doc.element(input).unwrap().attr("value"),
            Some("новое значение")
        );
        doc.set_attr(input, "placeholder", "Введите имя");
        assert_eq!(
            doc.element(input).unwrap().attr("placeholder"),
            Some("Введите имя")
        );
        assert_eq!(doc.element(input).unwrap().attributes.len(), 3);

        doc.remove_attr(input, "placeholder");
        assert_eq!(doc.element(input).unwrap().attr("placeholder"), None);
        doc.remove_attr(input, "missing");

        let textarea = doc.create_element("textarea", Vec::new());
        let old_text = doc.create_text("старый текст");
        doc.append_child(doc.root(), textarea);
        doc.append_child(textarea, old_text);

        doc.set_text(textarea, "первая строка\nвторая строка");
        assert_eq!(doc.text_content(textarea), "первая строка\nвторая строка");
        assert_eq!(doc.get(textarea).unwrap().children.len(), 1);
        // Reuses the existing text node instead of growing the arena on every keystroke.
        assert_eq!(doc.get(textarea).unwrap().children[0], old_text);

        let empty_para = doc.create_element("p", Vec::new());
        doc.append_child(doc.root(), empty_para);
        doc.set_text(empty_para, "Кириллица");
        assert_eq!(doc.text_content(empty_para), "Кириллица");
        doc.set_text(empty_para, "Ещё");
        assert_eq!(doc.text_content(empty_para), "Ещё");
        assert_eq!(doc.get(empty_para).unwrap().children.len(), 1);

        // Non-element ids are ignored.
        doc.set_attr(9999, "x", "y");
        doc.set_text(9999, "ignored");
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

    #[test]
    fn append_remove_cycle_guard_and_reparenting() {
        let mut doc = Document::new();
        let parent = doc.create_element("div", Vec::new());
        let child = doc.create_element("section", Vec::new());
        let grandchild = doc.create_element("p", Vec::new());
        let other = doc.create_element("aside", Vec::new());

        assert!(doc.append_child(doc.root(), parent));
        assert!(doc.append_child(parent, child));
        assert!(doc.append_child(child, grandchild));
        assert!(doc.append_child(doc.root(), other));

        // Self-parenting and ancestor cycles are rejected cleanly.
        assert!(!doc.append_child(parent, parent));
        assert!(!doc.append_child(grandchild, parent));
        assert!(!doc.append_child(child, doc.root()));
        assert_eq!(doc.parent_node(parent), Some(doc.root()));

        // Moving child to another parent detaches it from old parent cleanly.
        assert!(doc.append_child(other, child));
        assert_eq!(doc.parent_node(child), Some(other));
        assert!(doc.get(parent).unwrap().children.is_empty());
        assert_eq!(doc.get(other).unwrap().children, vec![child]);

        assert!(!doc.remove_child(parent, child));
        assert!(doc.remove_child(other, child));
        assert_eq!(doc.parent_node(child), None);
        assert!(doc.get(other).unwrap().children.is_empty());
    }

    #[test]
    fn query_selector_class_list_style_and_inner_html_helpers() {
        let mut doc = crate::html::parse(
            "<html><head><title>Old</title>\
             <script type=\"application/ld+json\">{\"ignored\":true}</script>\
             <script src=\"/app.js\"></script>\
             <script>console.log('привет');</script>\
             </head><body>\
             <div id=\"app\" class=\"shell active\">\
               <ul class=\"list\"><li>First</li><li class=\"item special\">Second</li></ul>\
               <p data-kind=\"note\">Кириллица</p>\
             </div></body></html>",
        );

        assert_eq!(
            doc.collect_script_sources(),
            vec![
                (Some("/app.js".to_string()), None),
                (None, Some("console.log('привет');".to_string())),
            ]
        );

        let app = doc.get_element_by_id("app").unwrap();
        assert_eq!(doc.query_selector("#app"), Some(app));
        let special = doc.query_selector("div#app > ul.list > li.item.special").unwrap();
        assert_eq!(doc.text_content_of(special), "Second");
        assert_eq!(
            doc.query_selector("body li.special, p[data-kind=\"note\"]"),
            Some(special)
        );
        assert_eq!(doc.query_selector_all("#app li").len(), 2);
        assert!(doc.query_selector("").is_none());

        assert!(doc.class_list_contains(app, "active"));
        assert!(!doc.class_list_toggle(app, "active"));
        assert!(!doc.class_list_contains(app, "active"));
        assert!(doc.class_list_toggle(app, "active"));
        doc.class_list_add(app, "extra");
        doc.class_list_remove(app, "shell");
        assert!(doc.class_list_contains(app, "extra"));
        assert!(!doc.class_list_contains(app, "shell"));

        doc.style_prop_set(app, "backgroundColor", "rgb(10, 20, 30)");
        doc.style_prop_set(app, "display", "flex");
        assert_eq!(doc.style_prop_get(app, "background-color"), "rgb(10, 20, 30)");
        assert_eq!(doc.style_prop_get(app, "backgroundColor"), "rgb(10, 20, 30)");
        doc.style_prop_set(app, "display", "");
        assert_eq!(doc.style_prop_get(app, "display"), "");
        doc.style_prop_set(app, "background-color", "");
        assert!(doc.element(app).unwrap().attr("style").is_none());

        doc.set_document_title("Новое имя");
        assert_eq!(doc.find_title().as_deref(), Some("Новое имя"));

        let note = doc.query_selector("p[data-kind=note]").unwrap();
        assert!(doc.set_inner_html_fragment(note, "<span>Привет</span><br><em>Мир</em>"));
        assert_eq!(
            doc.serialize_inner_html(note),
            "<span>Привет</span><br><em>Мир</em>"
        );
        doc.set_text_content_of(note, "A & <B>");
        assert_eq!(doc.text_content_of(note), "A & <B>");
        assert_eq!(doc.serialize_inner_html(note), "A &amp; &lt;B&gt;");
    }

    #[test]
    fn inner_html_and_query_selector_remain_iterative_on_deep_trees() {
        let mut doc = Document::new();
        let mut curr = doc.root();
        for _ in 0..8_000 {
            let next = doc.create_element("div", Vec::new());
            assert!(doc.append_child(curr, next));
            curr = next;
        }
        let leaf = doc.create_element("span", vec![("id".into(), "deep-leaf".into())]);
        assert!(doc.append_child(curr, leaf));
        doc.set_text_content_of(leaf, "ok");

        assert_eq!(doc.query_selector("div span#deep-leaf"), Some(leaf));
        let html_out = doc.serialize_inner_html(curr);
        assert_eq!(html_out, "<span id=\"deep-leaf\">ok</span>");
    }
}
