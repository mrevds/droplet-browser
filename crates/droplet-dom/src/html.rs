use crate::arena::{Document, Node, NodeId, NodeKind};

pub fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    let mut stack: Vec<NodeId> = vec![doc.root()];
    let bytes = source.as_bytes();
    let mut pos = 0;

    while pos < source.len() {
        let Some(rel_lt) = source[pos..].find('<') else {
            append_text(&mut doc, *stack.last().unwrap(), &decode_entities(&source[pos..]));
            break;
        };

        if rel_lt > 0 {
            let chunk = &source[pos..pos + rel_lt];
            append_text(&mut doc, *stack.last().unwrap(), &decode_entities(chunk));
            pos += rel_lt;
        }

        let rest = &source[pos..];

        if rest.starts_with("<!--") {
            if rest.starts_with("<!-->") {
                pos += 5;
            } else if rest.starts_with("<!--->") {
                pos += 6;
            } else if let Some(end_rel) = source[pos + 4..].find("-->") {
                pos = pos + 4 + end_rel + 3;
            } else {
                pos = source.len();
            }
            continue;
        }

        if rest.starts_with("<!") || rest.starts_with("<?") {
            if let Some(gt_rel) = source[pos + 2..].find('>') {
                pos = pos + 2 + gt_rel + 1;
            } else {
                pos = source.len();
            }
            continue;
        }

        if rest.starts_with("</") {
            pos += 2;
            if pos < bytes.len() && bytes[pos].is_ascii_alphabetic() {
                let name_start = pos;
                while pos < bytes.len()
                    && !bytes[pos].is_ascii_whitespace()
                    && bytes[pos] != b'>'
                    && bytes[pos] != b'/'
                {
                    pos += 1;
                }
                let tag_name = source[name_start..pos].to_ascii_lowercase();
                if let Some(gt_rel) = source[pos..].find('>') {
                    pos += gt_rel + 1;
                } else {
                    pos = source.len();
                }
                close_tag(&mut stack, &doc, &tag_name);
            } else if let Some(gt_rel) = source[pos..].find('>') {
                pos += gt_rel + 1;
            } else {
                pos = source.len();
            }
            continue;
        }

        if bytes.get(pos + 1).is_some_and(|b| b.is_ascii_alphabetic()) {
            pos += 1;
            let name_start = pos;
            while pos < bytes.len()
                && !bytes[pos].is_ascii_whitespace()
                && bytes[pos] != b'>'
                && bytes[pos] != b'/'
            {
                pos += 1;
            }
            let tag_name = source[name_start..pos].to_ascii_lowercase();
            let (attrs, self_closing, next_pos) = parse_attributes(source, pos);
            pos = next_pos;

            auto_close_for_open_tag(&mut stack, &doc, &tag_name);

            let parent_id = *stack.last().unwrap();
            let elem_id = doc.create_element(&tag_name, attrs);
            doc.append_child(parent_id, elem_id);

            if is_raw_text_tag(&tag_name) {
                if !self_closing {
                    let (raw_text, after_close) = read_raw_text(source, pos, &tag_name);
                    if !raw_text.is_empty() {
                        let text_id = doc.create_text(raw_text);
                        doc.append_child(elem_id, text_id);
                    }
                    pos = after_close;
                }
            } else if !self_closing && !is_void_tag(&tag_name) {
                stack.push(elem_id);
            }
            continue;
        }

        append_text(&mut doc, *stack.last().unwrap(), "<");
        pos += 1;
    }

    doc
}

fn parse_attributes(source: &str, mut pos: usize) -> (Vec<(String, String)>, bool, usize) {
    let bytes = source.as_bytes();
    let mut attrs: Vec<(String, String)> = Vec::new();
    let mut self_closing = false;

    while pos < bytes.len() {
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if pos >= bytes.len() {
            break;
        }
        if bytes[pos] == b'>' {
            pos += 1;
            break;
        }
        if bytes[pos] == b'/' {
            if bytes.get(pos + 1) == Some(&b'>') {
                self_closing = true;
                pos += 2;
                break;
            }
            pos += 1;
            continue;
        }

        let name_start = pos;
        while pos < bytes.len()
            && !bytes[pos].is_ascii_whitespace()
            && !matches!(bytes[pos], b'=' | b'>' | b'/')
        {
            pos += 1;
        }
        let attr_name = source[name_start..pos].to_ascii_lowercase();

        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }

        let attr_val = if pos < bytes.len() && bytes[pos] == b'=' {
            pos += 1;
            while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
                pos += 1;
            }
            if pos < bytes.len() && (bytes[pos] == b'"' || bytes[pos] == b'\'') {
                let quote = bytes[pos] as char;
                pos += 1;
                let val_start = pos;
                if let Some(end_rel) = source[pos..].find(quote) {
                    let raw_val = &source[val_start..pos + end_rel];
                    pos += end_rel + 1;
                    decode_entities(raw_val)
                } else {
                    let raw_val = &source[val_start..];
                    pos = source.len();
                    decode_entities(raw_val)
                }
            } else {
                let val_start = pos;
                while pos < bytes.len()
                    && !bytes[pos].is_ascii_whitespace()
                    && bytes[pos] != b'>'
                {
                    pos += 1;
                }
                decode_entities(&source[val_start..pos])
            }
        } else {
            String::new()
        };

        if !attr_name.is_empty() && !attrs.iter().any(|(existing, _)| existing == &attr_name) {
            attrs.push((attr_name, attr_val));
        }
    }

    (attrs, self_closing, pos)
}

fn read_raw_text<'a>(source: &'a str, start: usize, tag_name: &str) -> (&'a str, usize) {
    let mut search = start;
    while let Some(rel) = source[search..].find("</") {
        let candidate = search + rel;
        let after = &source[candidate + 2..];
        if after.len() >= tag_name.len()
            && after.as_bytes()[..tag_name.len()].eq_ignore_ascii_case(tag_name.as_bytes())
        {
            let next_byte = after.as_bytes().get(tag_name.len()).copied();
            if matches!(
                next_byte,
                None | Some(b'>')
                    | Some(b'/')
                    | Some(b' ')
                    | Some(b'\t')
                    | Some(b'\n')
                    | Some(b'\r')
            ) {
                let end_pos = match after[tag_name.len()..].find('>') {
                    Some(gt_rel) => candidate + 2 + tag_name.len() + gt_rel + 1,
                    None => source.len(),
                };
                return (&source[start..candidate], end_pos);
            }
        }
        search = candidate + 2;
    }
    (&source[start..], source.len())
}

fn append_text(doc: &mut Document, parent_id: NodeId, text: &str) {
    if text.is_empty() {
        return;
    }
    if let Some(&last_child) = doc.nodes.get(parent_id).and_then(|n| n.children.last()) {
        if let Some(Node {
            kind: NodeKind::Text(existing),
            ..
        }) = doc.nodes.get_mut(last_child)
        {
            existing.push_str(text);
            return;
        }
    }
    let text_id = doc.create_text(text);
    doc.append_child(parent_id, text_id);
}

fn close_tag(stack: &mut Vec<NodeId>, doc: &Document, tag_name: &str) {
    for i in (1..stack.len()).rev() {
        if doc
            .element(stack[i])
            .is_some_and(|el| el.tag_name == tag_name)
        {
            stack.truncate(i);
            return;
        }
    }
}

fn auto_close_for_open_tag(stack: &mut Vec<NodeId>, doc: &Document, tag_name: &str) {
    if closes_p(tag_name) {
        for i in (1..stack.len()).rev() {
            let Some(el) = doc.element(stack[i]) else {
                continue;
            };
            if el.tag_name == "p" {
                stack.truncate(i);
                break;
            }
            if is_p_scope_boundary(&el.tag_name) {
                break;
            }
        }
    }

    match tag_name {
        "li" => auto_close_until(stack, doc, &["li"], &["ul", "ol", "menu", "body", "html"]),
        "dt" | "dd" => auto_close_until(stack, doc, &["dt", "dd"], &["dl", "body", "html"]),
        "tr" => auto_close_until(stack, doc, &["tr"], &["table", "thead", "tbody", "tfoot"]),
        "td" | "th" => auto_close_until(stack, doc, &["td", "th"], &["tr", "table"]),
        "option" => auto_close_until(stack, doc, &["option"], &["select", "optgroup"]),
        _ => {}
    }
}

fn auto_close_until(
    stack: &mut Vec<NodeId>,
    doc: &Document,
    targets: &[&str],
    boundaries: &[&str],
) {
    for i in (1..stack.len()).rev() {
        let Some(el) = doc.element(stack[i]) else {
            continue;
        };
        if targets.iter().any(|&t| el.tag_name == t) {
            stack.truncate(i);
            return;
        }
        if boundaries.iter().any(|&b| el.tag_name == b) {
            return;
        }
    }
}

pub(crate) fn is_void_tag(tag: &str) -> bool {
    matches!(
        tag,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

fn is_raw_text_tag(tag: &str) -> bool {
    matches!(tag, "style" | "script")
}

fn closes_p(tag: &str) -> bool {
    matches!(
        tag,
        "address"
            | "article"
            | "aside"
            | "blockquote"
            | "dd"
            | "details"
            | "dialog"
            | "div"
            | "dl"
            | "dt"
            | "fieldset"
            | "figcaption"
            | "figure"
            | "footer"
            | "form"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "header"
            | "hgroup"
            | "hr"
            | "li"
            | "main"
            | "nav"
            | "ol"
            | "p"
            | "pre"
            | "section"
            | "table"
            | "ul"
    )
}

fn is_p_scope_boundary(tag: &str) -> bool {
    matches!(
        tag,
        "html"
            | "body"
            | "div"
            | "li"
            | "section"
            | "article"
            | "aside"
            | "header"
            | "footer"
            | "nav"
            | "main"
            | "blockquote"
            | "td"
            | "th"
    )
}

fn decode_entities(input: &str) -> String {
    if !input.contains('&') {
        return input.to_string();
    }

    let mut out = String::with_capacity(input.len());
    let mut pos = 0;

    while pos < input.len() {
        let Some(rel_amp) = input[pos..].find('&') else {
            out.push_str(&input[pos..]);
            break;
        };
        out.push_str(&input[pos..pos + rel_amp]);
        let amp_pos = pos + rel_amp;

        let after_amp = &input[amp_pos + 1..];
        let decoded = after_amp.find(';').and_then(|semi_rel| {
            if semi_rel == 0 || semi_rel > 16 {
                return None;
            }
            let body = &after_amp[..semi_rel];
            if body
                .bytes()
                .any(|b| b.is_ascii_whitespace() || b == b'&' || b == b'<')
            {
                return None;
            }
            decode_single_entity(body).map(|ch| (ch, amp_pos + 1 + semi_rel + 1))
        });

        match decoded {
            Some((ch, next_pos)) => {
                out.push(ch);
                pos = next_pos;
            }
            None => {
                out.push('&');
                pos = amp_pos + 1;
            }
        }
    }

    out
}

fn decode_single_entity(entity: &str) -> Option<char> {
    if let Some(hex) = entity.strip_prefix("#x").or_else(|| entity.strip_prefix("#X")) {
        if hex.is_empty() {
            return None;
        }
        let code = u32::from_str_radix(hex, 16).ok()?;
        return Some(sanitize_codepoint(code));
    }

    if let Some(dec) = entity.strip_prefix('#') {
        if dec.is_empty() {
            return None;
        }
        let code = dec.parse::<u32>().ok()?;
        return Some(sanitize_codepoint(code));
    }

    match entity {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some('\u{00A0}'),
        "mdash" => Some('\u{2014}'),
        "ndash" => Some('\u{2013}'),
        "rarr" => Some('\u{2192}'),
        "larr" => Some('\u{2190}'),
        "uarr" => Some('\u{2191}'),
        "darr" => Some('\u{2193}'),
        "bull" => Some('\u{2022}'),
        "laquo" => Some('\u{00AB}'),
        "raquo" => Some('\u{00BB}'),
        "copy" => Some('\u{00A9}'),
        "reg" => Some('\u{00AE}'),
        "trade" => Some('\u{2122}'),
        "hellip" => Some('\u{2026}'),
        "middot" => Some('\u{00B7}'),
        "times" => Some('\u{00D7}'),
        "divide" => Some('\u{00F7}'),
        "lsquo" => Some('\u{2018}'),
        "rsquo" => Some('\u{2019}'),
        "ldquo" => Some('\u{201C}'),
        "rdquo" => Some('\u{201D}'),
        "euro" => Some('\u{20AC}'),
        _ => None,
    }
}

fn sanitize_codepoint(code: u32) -> char {
    if code == 0 {
        '\u{FFFD}'
    } else {
        char::from_u32(code).unwrap_or('\u{FFFD}')
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_document_and_attributes() {
        let html = r#"<!DOCTYPE html>
        <!-- Header comment -->
        <html>
          <head><title>Droplet &mdash; Тест</title></head>
          <body>
            <div id="main" class='box primary' data-num=42 hidden>
              <img src="pic.png" alt="Картинка" />
              <br>
              <a href=about:roadmap>Дорожная карта &rarr;</a>
            </div>
          </body>
        </html>"#;

        let doc = parse(html);
        assert_eq!(doc.find_title().as_deref(), Some("Droplet — Тест"));

        let div_id = doc.find_first_tag("div").unwrap();
        let div = doc.element(div_id).unwrap();
        assert_eq!(div.id(), Some("main"));
        assert!(div.has_class("box"));
        assert!(div.has_class("primary"));
        assert_eq!(div.attr("data-num"), Some("42"));
        assert_eq!(div.attr("hidden"), Some(""));

        let img_id = doc.find_first_tag("img").unwrap();
        let img = doc.element(img_id).unwrap();
        assert_eq!(img.attr("alt"), Some("Картинка"));
        assert!(doc.get(img_id).unwrap().children.is_empty());

        let a_id = doc.find_first_tag("a").unwrap();
        assert_eq!(doc.element(a_id).unwrap().attr("href"), Some("about:roadmap"));
        assert_eq!(doc.text_content(a_id), "Дорожная карта →");
    }

    #[test]
    fn parse_raw_text_style_with_angle_brackets_and_cyrillic() {
        let html = r#"
        <style type="text/css">
          /* Комментарий с <тегами> и </notstyle> */
          div > p { color: #112233; }
          .a < .b { content: "x < y"; }
        </STYLE >
        <script>
          if (a < b && c > d) { console.log("Привет"); }
        </script>
        <p>После стилей</p>
        "#;

        let doc = parse(html);
        let css = doc.collect_style_text();
        assert!(css.contains("div > p { color: #112233; }"));
        assert!(css.contains("/* Комментарий с <тегами> и </notstyle> */"));

        let p_id = doc.find_first_tag("p").unwrap();
        assert_eq!(doc.text_content(p_id), "После стилей");
    }

    #[test]
    fn auto_close_p_and_li_and_recover_mismatched_tags() {
        let html = r#"
        <div>
          <p>Первый абзац
          <p>Второй абзац
          <div>Внутри блока</div>
          <ul>
            <li>Пункт 1
            <li>Пункт 2
              <ul>
                <li>Вложенный
              </ul>
            <li>Пункт 3
          </ul>
          <span>Текст</span></b></i>
        </div>
        "#;

        let doc = parse(html);
        let div_id = doc.find_first_tag("div").unwrap();
        let children = &doc.get(div_id).unwrap().children;

        let element_children: Vec<NodeId> = children
            .iter()
            .copied()
            .filter(|&cid| doc.element(cid).is_some())
            .collect();
        let tag_names: Vec<&str> = element_children
            .iter()
            .filter_map(|&cid| doc.element(cid).map(|e| e.tag_name.as_str()))
            .collect();
        assert_eq!(tag_names, vec!["p", "p", "div", "ul", "span"]);

        let outer_ul = element_children[3];
        let li_count = doc
            .get(outer_ul)
            .unwrap()
            .children
            .iter()
            .filter(|&&cid| doc.element(cid).is_some_and(|e| e.tag_name == "li"))
            .count();
        assert_eq!(li_count, 3);
    }

    #[test]
    fn entity_decoding_and_utf8_safety() {
        let html = concat!(
            "<p title=\"&laquo;Привет&raquo;\">",
            "&amp;lt; &lt;&gt;&quot;&#39;&apos;&nbsp;&ndash;&bull;&#1055;&#x0420; ",
            "AT&T 2 < 5 &unknown; &#x; &#99999999; Кириллица 🚀",
            "</p>"
        );
        let doc = parse(html);
        let p_id = doc.find_first_tag("p").unwrap();
        assert_eq!(doc.element(p_id).unwrap().attr("title"), Some("«Привет»"));

        let text = doc.text_content(p_id);
        assert!(text.contains("&lt; <>\"''\u{00A0}–•ПР"));
        assert!(text.contains("AT&T 2 < 5 &unknown; &#x; \u{FFFD} Кириллица 🚀"));
    }

    #[test]
    fn deeply_nested_html_does_not_stack_overflow() {
        let mut html = String::new();
        for _ in 0..5_000 {
            html.push_str("<div>");
        }
        html.push_str("ok");
        for _ in 0..5_000 {
            html.push_str("</div>");
        }
        let doc = parse(&html);
        assert_eq!(doc.text_content(doc.root()), "ok");
    }

    #[test]
    fn duplicate_attributes_keep_first_and_empty_tags_zero_alloc() {
        let html = r#"
            <p>Plain paragraph</p>
            <div id="first" ID="second" class="one" CLASS="two">
              <link rel="stylesheet" href="/app.css">
            </div>
        "#;
        let doc = parse(html);
        let p_id = doc.find_first_tag("p").unwrap();
        assert_eq!(doc.element(p_id).unwrap().attributes.capacity(), 0);

        let div_id = doc.find_first_tag("div").unwrap();
        let div = doc.element(div_id).unwrap();
        assert_eq!(div.id(), Some("first"));
        assert_eq!(div.attr("class"), Some("one"));
        assert_eq!(div.attributes.len(), 2);

        assert_eq!(doc.collect_stylesheet_links(), vec!["/app.css"]);
    }
}
