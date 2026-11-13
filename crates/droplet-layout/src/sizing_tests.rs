//! DROP-124: `box-sizing` and `min-*`/`max-*` in block, inline-block and flex sizing.

use std::collections::HashMap;

use droplet_dom::html;
use droplet_style::{css, style_tree};

use crate::{layout_tree, LayoutBox};

fn find<'a>(root: &'a LayoutBox, doc: &droplet_dom::Document, id: &str) -> &'a LayoutBox {
    let mut stack = vec![root];
    while let Some(b) = stack.pop() {
        if b.node_id
            .and_then(|n| doc.element(n))
            .and_then(|e| e.attr("id"))
            == Some(id)
        {
            return b;
        }
        stack.extend(b.children.iter());
    }
    panic!("no box #{id}");
}

/// Lays out `html_src` with `css_src` in an 800×600 viewport and returns the boxes for `ids`
/// as (border-box width, border-box height, border-box x).
fn boxes(css_src: &str, html_src: &str, ids: &[&str]) -> Vec<(f32, f32, f32)> {
    let doc = html::parse(html_src);
    let sheet = css::parse(css_src);
    let styled = style_tree(&doc, &sheet).expect("styled");
    let root = layout_tree(&doc, &styled, 800.0, 600.0, &HashMap::new());
    ids.iter()
        .map(|id| {
            let bb = find(&root, &doc, id).dimensions.border_box();
            (bb.width, bb.height, bb.x)
        })
        .collect()
}

const BASE: &str = "div{display:block} .p{width:600px;border:1px solid #000} ";

#[test]
fn border_box_width_includes_padding_and_border() {
    let css = format!(
        "{BASE} #bb{{box-sizing:border-box;width:100%;padding:20px;border:5px solid}} \
         #cb{{width:100%;padding:20px;border:5px solid}}"
    );
    let r = boxes(
        &css,
        r#"<div class="p"><div id="bb">border-box</div><div id="cb">content-box</div></div>"#,
        &["bb", "cb"],
    );
    assert_eq!(r[0].0, 600.0, "border-box fills the parent exactly");
    assert_eq!(r[1].0, 650.0, "content-box overflows by padding + border");
}

#[test]
fn border_box_px_width_and_height() {
    let css = format!(
        "{BASE} #a{{box-sizing:border-box;width:200px;height:100px;padding:10px 30px;border:2px solid}}"
    );
    let r = boxes(
        &css,
        r#"<div class="p"><div id="a">Привет</div></div>"#,
        &["a"],
    );
    assert_eq!(r[0].0, 200.0);
    assert_eq!(r[0].1, 100.0);
    // Padding larger than the width: content clamps at 0, box grows to padding + border.
    let css = format!("{BASE} #a{{box-sizing:border-box;width:10px;padding:0 20px}}");
    let r = boxes(&css, r#"<div class="p"><div id="a"></div></div>"#, &["a"]);
    assert_eq!(r[0].0, 40.0);
}

#[test]
fn max_and_min_width_clamp_in_css21_order() {
    let html = r#"<div class="p"><div id="max">max</div><div id="min">min</div><div id="both">both</div><div id="pct">pct</div></div>"#;
    let css = format!(
        "{BASE} #max{{max-width:200px}} #min{{width:50px;min-width:120px}} \
         #both{{width:300px;max-width:100px;min-width:250px}} #pct{{max-width:25%;padding:0 10px}}"
    );
    let r = boxes(&css, html, &["max", "min", "both", "pct"]);
    assert_eq!(r[0].0, 200.0);
    assert_eq!(r[1].0, 120.0);
    assert_eq!(r[2].0, 250.0, "min-width wins over max-width");
    assert_eq!(r[3].0, 170.0, "25% of 600 + padding");
}

#[test]
fn min_max_respect_border_box() {
    let css = format!(
        "{BASE} #a{{box-sizing:border-box;max-width:200px;padding:0 50px}} \
         #b{{box-sizing:border-box;width:10px;min-width:150px;padding:0 25px}}"
    );
    let r = boxes(
        &css,
        r#"<div class="p"><div id="a">a</div><div id="b">b</div></div>"#,
        &["a", "b"],
    );
    assert_eq!(r[0].0, 200.0);
    assert_eq!(r[1].0, 150.0);
}

#[test]
fn auto_margins_center_after_clamping() {
    let css = format!("{BASE} #c{{max-width:300px;margin:0 auto}} #d{{box-sizing:border-box;width:300px;padding:10px;margin:0 auto}}");
    let r = boxes(
        &css,
        r#"<div class="p"><div id="c">центр</div><div id="d">d</div></div>"#,
        &["c", "d"],
    );
    // Parent content box starts at x = 1 (border); (600 - 300) / 2 = 150.
    assert_eq!(r[0].0, 300.0);
    assert_eq!(r[0].2, 151.0);
    assert_eq!(r[1].0, 300.0);
    assert_eq!(r[1].2, 151.0);
}

#[test]
fn min_and_max_height() {
    let html = r#"<div class="p"><div id="a">a</div><div id="b">b</div><div id="c">c</div><div id="d">d</div></div>"#;
    let css = format!(
        "{BASE} #a{{min-height:80px}} #b{{height:300px;max-height:50px}} \
         #c{{box-sizing:border-box;min-height:80px;padding:10px}} #d{{height:10px;min-height:40px;max-height:20px}}"
    );
    let r = boxes(&css, html, &["a", "b", "c", "d"]);
    assert_eq!(r[0].1, 80.0);
    assert_eq!(r[1].1, 50.0);
    assert_eq!(r[2].1, 80.0);
    assert_eq!(r[3].1, 40.0, "min-height wins over max-height");
}

#[test]
fn inline_block_border_box_and_max_width() {
    let css = "span{display:inline-block} #a{box-sizing:border-box;width:100px;padding:0 20px} \
               #b{max-width:60px;padding:0 5px} #c{min-width:150px}";
    let r = boxes(
        css,
        r#"<div><span id="a">x</span><span id="b">очень длинный текст здесь</span><span id="c">c</span></div>"#,
        &["a", "b", "c"],
    );
    assert_eq!(r[0].0, 100.0);
    assert_eq!(r[1].0, 70.0);
    assert_eq!(r[2].0, 150.0);
}

#[test]
fn flex_items_respect_box_sizing_and_min_max() {
    let css = "div{display:block} #f{display:flex;width:600px} \
               #a{box-sizing:border-box;width:200px;padding:0 20px;flex-shrink:0} \
               #b{flex-grow:1;max-width:100px} #c{flex-grow:1}";
    let r = boxes(
        css,
        r#"<div id="f"><div id="a">a</div><div id="b">b</div><div id="c">c</div></div>"#,
        &["a", "b", "c"],
    );
    assert_eq!(r[0].0, 200.0);
    assert_eq!(r[1].0, 100.0, "grow is capped by max-width");
    assert!(
        r[2].0 > 250.0,
        "the rest of the space goes to #c: {}",
        r[2].0
    );

    // Shrinking stops at min-width.
    let css = "div{display:block} #f{display:flex;width:300px} \
               #a{width:250px;min-width:200px} #b{width:250px}";
    let r = boxes(
        css,
        r#"<div id="f"><div id="a">a</div><div id="b">b</div></div>"#,
        &["a", "b"],
    );
    assert_eq!(r[0].0, 200.0);
}

#[test]
fn border_box_children_get_border_box_percent_of_content() {
    // A border-box parent's percentage children resolve against its *content* width.
    let css =
        "div{display:block} #p{box-sizing:border-box;width:400px;padding:0 50px} #c{width:50%}";
    let r = boxes(css, r#"<div id="p"><div id="c">c</div></div>"#, &["p", "c"]);
    assert_eq!(r[0].0, 400.0);
    assert_eq!(r[1].0, 150.0);
}
