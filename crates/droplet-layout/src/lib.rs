mod block;
mod box_model;
mod flex;
mod forms;
mod hit_test;
mod inline;
mod replaced;
#[cfg(test)]
mod sizing_tests;

use std::collections::HashMap;
use std::sync::Arc;

use droplet_dom::Document;
use droplet_gfx::image::DecodedImage;
use droplet_style::StyledNode;

pub use droplet_gfx::Rect;
pub use droplet_style::EdgeSizes;

pub use box_model::{dump_layout, find_layout_box_by_node, BoxType, Dimensions, LayoutBox};
pub use forms::{control_kind, control_snapshot, control_value, selected_option, ControlBox, ControlKind};
pub use hit_test::{hit_test, hit_test_href, hit_test_node};

use box_model::{box_dispatch, LayoutCtx};

pub fn layout_tree(
    doc: &Document,
    styled_root: &StyledNode,
    viewport_width: f32,
    viewport_height: f32,
    images: &HashMap<String, Arc<DecodedImage>>,
) -> LayoutBox {
    let ctx = LayoutCtx { doc, images };
    let container = Rect {
        x: 0.0,
        y: 0.0,
        width: viewport_width.max(1.0),
        height: viewport_height.max(1.0),
    };
    box_dispatch(&ctx, styled_root, container, 0.0, None, None, None)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use droplet_dom::Document;
    use droplet_gfx::font;
    use droplet_gfx::image::DecodedImage;
    use droplet_style::css::{self, Stylesheet};
    use droplet_style::style_tree;

    use super::{
        dump_layout, find_layout_box_by_node, hit_test, hit_test_href, hit_test_node, layout_tree,
        BoxType, ControlKind, LayoutBox,
    };

    fn attrs(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn collect_text_boxes(node: &LayoutBox) -> Vec<&LayoutBox> {
        let mut out = Vec::new();
        if matches!(node.box_type, BoxType::Text { .. }) {
            out.push(node);
        }
        for c in &node.children {
            out.extend(collect_text_boxes(c));
        }
        out
    }

    fn collect_boxes_by_type<'a>(
        node: &'a LayoutBox,
        pred: &dyn Fn(&BoxType) -> bool,
    ) -> Vec<&'a LayoutBox> {
        let mut out = Vec::new();
        if pred(&node.box_type) {
            out.push(node);
        }
        for c in &node.children {
            out.extend(collect_boxes_by_type(c, pred));
        }
        out
    }

    #[test]
    fn test_margin_auto_and_max_width_centering() {
        let mut doc = Document::new();
        let container = doc.create_element(
            "div",
            attrs(&[(
                "style",
                "display: block; max-width: 500px; padding: 10px; margin: 0 auto;",
            )]),
        );
        let fixed = doc.create_element(
            "div",
            attrs(&[(
                "style",
                "display: block; width: 200px; margin: 10px auto; height: 20px;",
            )]),
        );
        doc.append_child(doc.root(), container);
        doc.append_child(container, fixed);

        let styled = style_tree(&doc, &Stylesheet::default()).unwrap();
        let laid = layout_tree(&doc, &styled, 800.0, 600.0, &HashMap::new());

        let c_box = &laid.children[0];
        assert_eq!(c_box.dimensions.content.width, 500.0);
        assert_eq!(c_box.dimensions.margin.left, 140.0);
        assert_eq!(c_box.dimensions.margin.right, 140.0);
        assert_eq!(c_box.dimensions.content.x, 150.0);

        let f_box = &c_box.children[0];
        assert_eq!(f_box.dimensions.margin.left, 150.0);
        assert_eq!(f_box.dimensions.content.x, 300.0);
    }

    #[test]
    fn test_inline_flow_wrapping_and_hit_test_href() {
        let mut doc = Document::new();
        let p = doc.create_element("p", attrs(&[("style", "margin: 0;")]));
        let t1 = doc.create_text("Read the ");
        let a = doc.create_element("a", attrs(&[("href", "about:roadmap")]));
        let a_txt = doc.create_text("Roadmap");
        let t2 = doc.create_text(" or ");
        let code = doc.create_element("code", HashMap::new());
        let c_txt = doc.create_text("Architecture");
        let t3 = doc.create_text(".");
        doc.append_child(doc.root(), p);
        doc.append_child(p, t1);
        doc.append_child(p, a);
        doc.append_child(a, a_txt);
        doc.append_child(p, t2);
        doc.append_child(p, code);
        doc.append_child(code, c_txt);
        doc.append_child(p, t3);

        let styled = style_tree(&doc, &Stylesheet::default()).unwrap();
        let laid = layout_tree(&doc, &styled, 180.0, 600.0, &HashMap::new());
        let texts = collect_text_boxes(&laid);

        let roadmap_box = texts
            .iter()
            .find(|b| matches!(&b.box_type, BoxType::Text { lines, .. } if lines.as_slice() == &["Roadmap"][..]))
            .expect("Roadmap box");
        let arch_box = texts
            .iter()
            .find(|b| matches!(&b.box_type, BoxType::Text { lines, .. } if lines.as_slice() == &["Architecture"][..]))
            .expect("Architecture box");

        let expected_x = font::measure_text_px("Read the ", 16.0, false, false);
        assert!((roadmap_box.dimensions.content.x - expected_x).abs() < 0.05);
        assert!(arch_box.dimensions.content.y > roadmap_box.dimensions.content.y);
        assert_eq!(arch_box.dimensions.padding.left, 4.0);
        assert!(arch_box.style.background_color.is_some());

        let rx = roadmap_box.dimensions.content.x + 10.0;
        let ry = roadmap_box.dimensions.content.y + 5.0;
        assert_eq!(hit_test_href(&laid, rx, ry), Some("about:roadmap"));
        assert_eq!(hit_test_href(&laid, 5.0, ry), None);
    }

    #[test]
    fn test_preformatted_preserves_lines_and_spaces() {
        let mut doc = Document::new();
        let pre = doc.create_element("pre", HashMap::new());
        let txt = doc.create_text("  fn main() {\n\n    println!(\"hi\");\n}");
        doc.append_child(doc.root(), pre);
        doc.append_child(pre, txt);

        let styled = style_tree(&doc, &Stylesheet::default()).unwrap();
        let laid = layout_tree(&doc, &styled, 600.0, 600.0, &HashMap::new());
        let texts = collect_text_boxes(&laid);
        assert_eq!(texts.len(), 1);
        if let BoxType::Text { lines, .. } = &texts[0].box_type {
            assert_eq!(
                lines.as_slice(),
                &["  fn main() {", "", "    println!(\"hi\");", "}"][..]
            );
        } else {
            panic!("expected Text box");
        }
    }

    #[test]
    fn test_utf8_cyrillic_word_wrapping() {
        let mut doc = Document::new();
        let p = doc.create_element("p", HashMap::new());
        let txt = doc.create_text("Привет мир это тест переноса Длинноесловобезпробелов");
        doc.append_child(doc.root(), p);
        doc.append_child(p, txt);

        let styled = style_tree(&doc, &Stylesheet::default()).unwrap();
        let laid = layout_tree(&doc, &styled, 80.0, 600.0, &HashMap::new());
        let texts = collect_text_boxes(&laid);
        assert_eq!(texts.len(), 1);
        if let BoxType::Text { lines, .. } = &texts[0].box_type {
            assert!(lines.len() >= 4);
            for line in lines {
                assert!(font::measure_text_px(line, 16.0, false, false) <= 80.05);
            }
            let rejoined: String = lines.join("");
            assert!(rejoined.contains("Привет"));
            assert!(rejoined.contains("Длинноесловобезпробелов"));
        } else {
            panic!("expected Text box");
        }
    }

    #[test]
    fn test_dump_layout_contains_boxes_and_href() {
        let mut doc = Document::new();
        let a = doc.create_element("a", attrs(&[("href", "https://example.org")]));
        let t = doc.create_text("Example");
        doc.append_child(doc.root(), a);
        doc.append_child(a, t);

        let styled = style_tree(&doc, &css::parse("")).unwrap();
        let laid = layout_tree(&doc, &styled, 400.0, 600.0, &HashMap::new());
        let dump = dump_layout(&laid);
        assert!(dump.contains("Block"));
        assert!(dump.contains("Text"));
        assert!(dump.contains("href=\"https://example.org\""));
        assert!(dump.contains("\"Example\""));
    }

    #[test]
    fn test_inline_block_side_by_side_and_wrapping() {
        let mut doc = Document::new();
        let box1 = doc.create_element(
            "div",
            attrs(&[("style", "display: inline-block; width: 100px; height: 30px;")]),
        );
        let box2 = doc.create_element(
            "div",
            attrs(&[("style", "display: inline-block; width: 100px; height: 30px;")]),
        );
        doc.append_child(doc.root(), box1);
        doc.append_child(doc.root(), box2);

        let styled = style_tree(&doc, &Stylesheet::default()).unwrap();

        let wide = layout_tree(&doc, &styled, 250.0, 400.0, &HashMap::new());
        let wide_ibs = collect_boxes_by_type(&wide, &|bt| matches!(bt, BoxType::InlineBlock));
        assert_eq!(wide_ibs.len(), 2);
        assert_eq!(wide_ibs[0].dimensions.content.y, wide_ibs[1].dimensions.content.y);
        assert_eq!(wide_ibs[0].dimensions.content.x, 0.0);
        assert_eq!(wide_ibs[1].dimensions.content.x, 100.0);

        let narrow = layout_tree(&doc, &styled, 150.0, 400.0, &HashMap::new());
        let narrow_ibs = collect_boxes_by_type(&narrow, &|bt| matches!(bt, BoxType::InlineBlock));
        assert_eq!(narrow_ibs.len(), 2);
        assert!(narrow_ibs[1].dimensions.content.y >= narrow_ibs[0].dimensions.content.y + 30.0);
        assert_eq!(narrow_ibs[1].dimensions.content.x, 0.0);
    }

    #[test]
    fn test_flexbox_row_space_between_align_center_and_gap() {
        let mut doc = Document::new();
        let flex = doc.create_element(
            "div",
            attrs(&[(
                "style",
                "display: flex; justify-content: space-between; align-items: center; gap: 12px;",
            )]),
        );
        let c1 = doc.create_element("div", attrs(&[("style", "width: 100px; height: 40px;")]));
        let c2 = doc.create_element("div", attrs(&[("style", "width: 100px; height: 80px;")]));
        doc.append_child(doc.root(), flex);
        doc.append_child(flex, c1);
        doc.append_child(flex, c2);

        let styled = style_tree(&doc, &Stylesheet::default()).unwrap();
        let laid = layout_tree(&doc, &styled, 400.0, 400.0, &HashMap::new());
        let flex_box = &laid.children[0];
        assert_eq!(flex_box.box_type, BoxType::Flex);
        assert_eq!(flex_box.children.len(), 2);

        let b1 = &flex_box.children[0];
        let b2 = &flex_box.children[1];
        assert_eq!(b1.dimensions.content.x, 0.0);
        assert_eq!(b1.dimensions.content.y, 20.0);
        assert_eq!(b2.dimensions.content.x, 300.0);
        assert_eq!(b2.dimensions.content.y, 0.0);
    }

    #[test]
    fn test_flexbox_grow_shrink_clamp_wrap_and_column() {
        // flex-grow + gap + flex-shrink clamp >= 0
        let mut doc = Document::new();
        let row_grow = doc.create_element(
            "div",
            attrs(&[("style", "display: flex; width: 412px; gap: 12px;")]),
        );
        let g1 = doc.create_element("div", attrs(&[("style", "width: 100px; height: 20px;")]));
        let g2 = doc.create_element(
            "div",
            attrs(&[("style", "flex-grow: 1; flex-basis: 0px; height: 20px;")]),
        );
        doc.append_child(doc.root(), row_grow);
        doc.append_child(row_grow, g1);
        doc.append_child(row_grow, g2);

        let row_shrink = doc.create_element(
            "div",
            attrs(&[("style", "display: flex; width: 100px;")]),
        );
        let s1 = doc.create_element(
            "div",
            attrs(&[("style", "width: 200px; flex-shrink: 1; height: 20px;")]),
        );
        let s2 = doc.create_element(
            "div",
            attrs(&[("style", "width: 200px; flex-shrink: 1; height: 20px;")]),
        );
        doc.append_child(doc.root(), row_shrink);
        doc.append_child(row_shrink, s1);
        doc.append_child(row_shrink, s2);

        let row_clamp = doc.create_element(
            "div",
            attrs(&[("style", "display: flex; width: 40px;")]),
        );
        let sc1 = doc.create_element(
            "div",
            attrs(&[("style", "width: 50px; padding: 30px; flex-shrink: 1; height: 20px;")]),
        );
        doc.append_child(doc.root(), row_clamp);
        doc.append_child(row_clamp, sc1);

        let wrap_box = doc.create_element(
            "div",
            attrs(&[("style", "display: flex; flex-wrap: wrap; width: 250px; gap: 10px;")]),
        );
        for _ in 0..3 {
            let item =
                doc.create_element("div", attrs(&[("style", "width: 100px; height: 30px;")]));
            doc.append_child(wrap_box, item);
        }
        doc.append_child(doc.root(), wrap_box);

        let col_box = doc.create_element(
            "div",
            attrs(&[("style", "display: flex; flex-direction: column; width: 300px; gap: 10px;")]),
        );
        let col1 = doc.create_element("div", attrs(&[("style", "height: 40px;")]));
        let col2 = doc.create_element("div", attrs(&[("style", "height: 60px;")]));
        doc.append_child(doc.root(), col_box);
        doc.append_child(col_box, col1);
        doc.append_child(col_box, col2);

        let styled = style_tree(&doc, &Stylesheet::default()).unwrap();
        let laid = layout_tree(&doc, &styled, 800.0, 600.0, &HashMap::new());

        let fg = &laid.children[0];
        assert_eq!(fg.children[0].dimensions.content.width, 100.0);
        assert_eq!(fg.children[1].dimensions.content.width, 300.0);
        assert_eq!(fg.children[1].dimensions.content.x, 112.0);

        let fs = &laid.children[1];
        assert!(fs.children[0].dimensions.content.width >= 0.0);
        assert!(fs.children[1].dimensions.content.width >= 0.0);
        assert_eq!(fs.children[0].dimensions.content.width, 50.0);
        assert_eq!(fs.children[1].dimensions.content.width, 50.0);

        let fc_clamp = &laid.children[2];
        assert_eq!(fc_clamp.children[0].dimensions.content.width, 0.0);

        let fw = &laid.children[3];
        assert_eq!(fw.children[0].dimensions.content.x, 0.0);
        assert_eq!(fw.children[1].dimensions.content.x, 110.0);
        assert_eq!(fw.children[2].dimensions.content.x, 0.0);
        assert_eq!(
            fw.children[2].dimensions.content.y,
            fw.children[0].dimensions.content.y + 40.0
        );
        assert_eq!(fw.dimensions.content.height, 70.0);

        let fc = &laid.children[4];
        assert_eq!(fc.children[0].dimensions.content.width, 300.0);
        assert_eq!(fc.children[1].dimensions.content.width, 300.0);
        assert_eq!(
            fc.children[1].dimensions.content.y,
            fc.children[0].dimensions.content.y + 50.0
        );
        assert_eq!(fc.dimensions.content.height, 110.0);
    }

    #[test]
    fn test_image_aspect_ratio_and_alt_fallback() {
        let mut doc = Document::new();
        let img1 = doc.create_element(
            "img",
            attrs(&[("src", "logo.png"), ("width", "100"), ("alt", "Logo")]),
        );
        let img2 = doc.create_element(
            "img",
            attrs(&[("src", "missing.png"), ("alt", "Broken photo")]),
        );
        doc.append_child(doc.root(), img1);
        doc.append_child(doc.root(), img2);

        let mut images = HashMap::new();
        images.insert(
            "logo.png".to_string(),
            Arc::new(DecodedImage {
                width: 200,
                height: 100,
                rgba: vec![255; 200 * 100 * 4],
            }),
        );

        let styled = style_tree(&doc, &Stylesheet::default()).unwrap();
        let laid = layout_tree(&doc, &styled, 600.0, 400.0, &images);
        let imgs = collect_boxes_by_type(&laid, &|bt| matches!(bt, BoxType::Image { .. }));
        assert_eq!(imgs.len(), 2);

        assert_eq!(imgs[0].dimensions.content.width, 100.0);
        assert_eq!(imgs[0].dimensions.content.height, 50.0);
        assert!(matches!(
            &imgs[0].box_type,
            BoxType::Image { image: Some(_), .. }
        ));

        assert!(imgs[1].dimensions.content.width > 20.0);
        assert!(imgs[1].dimensions.content.height >= 20.0);
        assert!(matches!(
            &imgs[1].box_type,
            BoxType::Image { image: None, alt } if alt == "Broken photo"
        ));
    }

    #[test]
    fn test_hit_test_node_returns_deepest_with_ancestor_fallback() {
        let mut doc = Document::new();
        let card = doc.create_element(
            "div",
            attrs(&[("style", "display: block; width: 200px; height: 100px;")]),
        );
        let span = doc.create_element("span", HashMap::new());
        let txt = doc.create_text("Click");
        let block = doc.create_element(
            "div",
            attrs(&[("style", "display: block; width: 100px; height: 20px;")]),
        );
        doc.append_child(doc.root(), card);
        doc.append_child(card, span);
        doc.append_child(span, txt);
        doc.append_child(card, block);

        let styled = style_tree(&doc, &Stylesheet::default()).unwrap();
        let laid = layout_tree(&doc, &styled, 400.0, 400.0, &HashMap::new());

        // Clicking on text inside AnonymousBlock hits the text NodeId
        let texts = collect_text_boxes(&laid);
        let t_rect = texts[0].dimensions.content;
        assert_eq!(
            hit_test_node(&laid, t_rect.x + 2.0, t_rect.y + 2.0),
            Some(txt)
        );
        assert!(
            hit_test(&laid, t_rect.x + 2.0, t_rect.y + 2.0)
                .is_some_and(|b| b.node_id == Some(txt))
        );
        // Clicking empty space inside card (right of text, below block) falls back to card NodeId
        assert_eq!(hit_test_node(&laid, 180.0, 80.0), Some(card));
        // Clicking outside everything hits root Document (0) or None if outside root
        assert_eq!(hit_test_node(&laid, 500.0, 500.0), None);
    }

    #[test]
    fn test_find_layout_box_by_node_matches_deep_text_box() {
        let mut doc = Document::new();
        let card = doc.create_element(
            "div",
            attrs(&[("style", "display: block; width: 200px; height: 100px;")]),
        );
        let span = doc.create_element("span", HashMap::new());
        let txt = doc.create_text("Click");
        doc.append_child(doc.root(), card);
        doc.append_child(card, span);
        doc.append_child(span, txt);

        let styled = style_tree(&doc, &Stylesheet::default()).unwrap();
        let laid = layout_tree(&doc, &styled, 400.0, 400.0, &HashMap::new());

        let card_box = find_layout_box_by_node(&laid, card).expect("card box");
        assert_eq!(card_box.node_id, Some(card));
        assert!((card_box.dimensions.content.width - 200.0).abs() < 0.01);

        let text_box = find_layout_box_by_node(&laid, txt).expect("text box");
        assert!(matches!(text_box.box_type, BoxType::Text { .. }));
        assert!(find_layout_box_by_node(&laid, 9999).is_none());
    }

    #[test]
    fn test_percentage_sizing_ignores_auto_parent_height() {
        let mut doc = Document::new();
        let auto_parent = doc.create_element("div", attrs(&[("style", "width: 400px;")]));
        let pct_child = doc.create_element(
            "div",
            attrs(&[("style", "width: 50%; height: 50%;")]),
        );
        let def_parent = doc.create_element(
            "div",
            attrs(&[("style", "width: 400px; height: 200px;")]),
        );
        let def_child = doc.create_element(
            "div",
            attrs(&[("style", "width: 50%; height: 50%;")]),
        );
        doc.append_child(doc.root(), auto_parent);
        doc.append_child(auto_parent, pct_child);
        doc.append_child(doc.root(), def_parent);
        doc.append_child(def_parent, def_child);

        let styled = style_tree(&doc, &Stylesheet::default()).unwrap();
        let laid = layout_tree(&doc, &styled, 800.0, 600.0, &HashMap::new());

        let c_auto = &laid.children[0].children[0];
        assert_eq!(c_auto.dimensions.content.width, 200.0);
        assert_eq!(c_auto.dimensions.content.height, 0.0);

        let c_def = &laid.children[1].children[0];
        assert_eq!(c_def.dimensions.content.width, 200.0);
        assert_eq!(c_def.dimensions.content.height, 100.0);
    }

    #[test]
    fn test_form_controls_atomic_layout_and_password_masking() {
        let mut doc = Document::new();
        let form = doc.create_element("form", Vec::new());
        let txt = doc.create_element(
            "input",
            attrs(&[
                ("type", "text"),
                ("name", "user"),
                ("value", "Привет"),
                ("placeholder", "Имя"),
            ]),
        );
        let pwd = doc.create_element(
            "input",
            attrs(&[("type", "password"), ("name", "pw"), ("value", "секрет")]),
        );
        let chk = doc.create_element("input", attrs(&[("type", "checkbox"), ("checked", "")]));
        let rad = doc.create_element("input", attrs(&[("type", "radio")]));
        let sub = doc.create_element("input", attrs(&[("type", "submit"), ("value", "Отправить")]));
        let area = doc.create_element("textarea", Vec::new());
        let area_txt = doc.create_text("line1\nline2");
        let sel = doc.create_element("select", Vec::new());
        let opt1 = doc.create_element("option", attrs(&[("value", "a")]));
        let opt1_t = doc.create_text("First");
        let opt2 = doc.create_element("option", attrs(&[("value", "b"), ("selected", "")]));
        let opt2_t = doc.create_text("Выбор");
        let btn = doc.create_element("button", attrs(&[("disabled", "")]));
        let btn_t = doc.create_text("Go");

        doc.append_child(doc.root(), form);
        doc.append_child(form, txt);
        doc.append_child(form, pwd);
        doc.append_child(form, chk);
        doc.append_child(form, rad);
        doc.append_child(form, sub);
        doc.append_child(form, area);
        doc.append_child(area, area_txt);
        doc.append_child(form, sel);
        doc.append_child(sel, opt1);
        doc.append_child(opt1, opt1_t);
        doc.append_child(sel, opt2);
        doc.append_child(opt2, opt2_t);
        doc.append_child(form, btn);
        doc.append_child(btn, btn_t);

        let styled = style_tree(&doc, &Stylesheet::default()).unwrap();
        let laid = layout_tree(&doc, &styled, 800.0, 600.0, &HashMap::new());
        let controls = collect_boxes_by_type(&laid, &|bt| matches!(bt, BoxType::Control(_)));
        assert_eq!(controls.len(), 8);

        match &controls[0].box_type {
            BoxType::Control(c) => {
                assert_eq!(c.kind, ControlKind::TextInput);
                assert_eq!(c.text, "Привет");
                assert_eq!(c.placeholder, "Имя");
                assert!(controls[0].dimensions.content.width >= 100.0);
            }
            _ => unreachable!(),
        }
        match &controls[1].box_type {
            BoxType::Control(c) => {
                assert_eq!(c.kind, ControlKind::PasswordInput);
                assert_eq!(c.text, "\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}");
            }
            _ => unreachable!(),
        }
        match &controls[2].box_type {
            BoxType::Control(c) => {
                assert_eq!(c.kind, ControlKind::Checkbox);
                assert!(c.checked);
                assert_eq!(controls[2].dimensions.content.width, 14.0);
            }
            _ => unreachable!(),
        }
        match &controls[3].box_type {
            BoxType::Control(c) => {
                assert_eq!(c.kind, ControlKind::Radio);
                assert!(!c.checked);
            }
            _ => unreachable!(),
        }
        match &controls[4].box_type {
            BoxType::Control(c) => {
                assert_eq!(c.kind, ControlKind::Submit);
                assert_eq!(c.text, "Отправить");
            }
            _ => unreachable!(),
        }
        match &controls[5].box_type {
            BoxType::Control(c) => {
                assert_eq!(c.kind, ControlKind::Textarea);
                assert_eq!(c.text, "line1\nline2");
                assert!(controls[5].children.is_empty());
            }
            _ => unreachable!(),
        }
        match &controls[6].box_type {
            BoxType::Control(c) => {
                assert_eq!(c.kind, ControlKind::Select);
                assert_eq!(c.text, "Выбор");
                assert!(controls[6].children.is_empty());
            }
            _ => unreachable!(),
        }
        match &controls[7].box_type {
            BoxType::Control(c) => {
                assert_eq!(c.kind, ControlKind::Button);
                assert!(c.disabled);
                assert_eq!(c.text, "Go");
            }
            _ => unreachable!(),
        }
    }
}
