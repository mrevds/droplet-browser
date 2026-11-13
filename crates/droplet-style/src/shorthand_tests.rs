//! DROP-124: box-sizing, min/max sizes, insets and the shorthand audit.

use droplet_dom::html;

use crate::cascade::{style_tree, StyledNode};
use crate::computed::{BoxSizing, ComputedStyle, EdgeSizes, LengthPct};
use crate::css::{self, Color};

fn find<'a>(node: &'a StyledNode, doc: &droplet_dom::Document, id: &str) -> Option<&'a StyledNode> {
    let mut stack = vec![node];
    while let Some(n) = stack.pop() {
        if doc.element(n.node_id).and_then(|e| e.attr("id")) == Some(id) {
            return Some(n);
        }
        stack.extend(n.children.iter());
    }
    None
}

/// Style of `#t` in `<div id=t>` under `css`.
fn style_with(css_src: &str) -> ComputedStyle {
    styles_with(
        css_src,
        r#"<div id="p"><div id="t">Привет, мир</div></div>"#,
        "t",
    )
}

fn styles_with(css_src: &str, html_src: &str, id: &str) -> ComputedStyle {
    let doc = html::parse(html_src);
    let sheet = css::parse(css_src);
    let tree = style_tree(&doc, &sheet).expect("styled tree");
    find(&tree, &doc, id).expect("element").style
}

fn edges(t: f32, r: f32, b: f32, l: f32) -> EdgeSizes {
    EdgeSizes {
        top: t,
        right: r,
        bottom: b,
        left: l,
    }
}

#[test]
fn box_sizing_keywords_and_inherit() {
    assert_eq!(style_with("#t{}").box_sizing, BoxSizing::ContentBox);
    assert_eq!(
        style_with("#t{box-sizing:border-box}").box_sizing,
        BoxSizing::BorderBox
    );
    assert_eq!(
        style_with("#t{box-sizing:border-box;box-sizing:content-box}").box_sizing,
        BoxSizing::ContentBox
    );
    // Not inherited by default…
    assert_eq!(
        style_with("#p{box-sizing:border-box}").box_sizing,
        BoxSizing::ContentBox
    );
    // …but `inherit` (the `html{border-box} *{inherit}` idiom) works.
    let s = style_with("#p{box-sizing:border-box} *{box-sizing:inherit}");
    assert_eq!(s.box_sizing, BoxSizing::BorderBox);
    // Invalid keyword ignored.
    assert_eq!(
        style_with("#t{box-sizing:border-box;box-sizing:padding-box}").box_sizing,
        BoxSizing::BorderBox
    );
}

#[test]
fn min_max_sizes_parse_lengths_percent_and_keywords() {
    let s = style_with("#t{min-width:10em;min-height:50%;max-height:120px}");
    assert_eq!(s.min_width, LengthPct::Px(160.0));
    assert_eq!(s.min_height, LengthPct::Percent(50.0));
    assert_eq!(s.max_height, LengthPct::Px(120.0));

    let s = style_with("#t{min-width:10px;min-width:auto;max-height:5px;max-height:none}");
    assert_eq!(s.min_width, LengthPct::Auto);
    assert_eq!(s.max_height, LengthPct::Auto);

    // Negative values and garbage are invalid: the earlier value survives.
    let s = style_with("#t{min-width:20px;min-width:-5px;max-height:30px;max-height:big}");
    assert_eq!(s.min_width, LengthPct::Px(20.0));
    assert_eq!(s.max_height, LengthPct::Px(30.0));
    assert_eq!(s.max_width, LengthPct::Auto);
    let s = style_with("#t{max-width:40%}");
    assert_eq!(s.max_width, LengthPct::Percent(40.0));
}

#[test]
fn margin_and_padding_one_to_four_values() {
    let cases = [
        ("1px", edges(1.0, 1.0, 1.0, 1.0)),
        ("1px 2px", edges(1.0, 2.0, 1.0, 2.0)),
        ("1px 2px 3px", edges(1.0, 2.0, 3.0, 2.0)),
        ("1px 2px 3px 4px", edges(1.0, 2.0, 3.0, 4.0)),
        ("0 1em", edges(0.0, 16.0, 0.0, 16.0)),
    ];
    for (v, want) in cases {
        assert_eq!(
            style_with(&format!("#t{{padding:{v}}}")).padding,
            want,
            "padding {v}"
        );
        assert_eq!(
            style_with(&format!("#t{{margin:{v}}}")).margin,
            want,
            "margin {v}"
        );
    }
    // Five values, negative padding, unknown units: whole declaration ignored.
    for bad in [
        "1px 2px 3px 4px 5px",
        "-1px",
        "1px -2px",
        "1px foo",
        "3furlongs",
    ] {
        let s = style_with(&format!("#t{{padding:7px; padding:{bad}}}"));
        assert_eq!(s.padding, edges(7.0, 7.0, 7.0, 7.0), "padding {bad}");
    }
    // Negative margins are fine; `auto` sides flagged.
    let s = style_with("#t{margin:-4px auto}");
    assert_eq!(s.margin.top, -4.0);
    assert!(s.margin_left_auto && s.margin_right_auto);
    // Longhand after shorthand overrides one side.
    let s = style_with("#t{padding:4px;padding-left:9px;padding-top:-3px}");
    assert_eq!(s.padding, edges(4.0, 4.0, 4.0, 9.0));
}

#[test]
fn border_shorthand_resets_and_needs_a_style() {
    let s = style_with("#t{border:2px solid #f00}");
    assert_eq!(s.border, edges(2.0, 2.0, 2.0, 2.0));
    assert_eq!(s.border_color, Color::rgb(255, 0, 0));

    // Any order.
    let s = style_with("#t{border:#00f dashed thin}");
    assert_eq!(s.border, edges(1.0, 1.0, 1.0, 1.0));
    assert_eq!(s.border_color, Color::rgb(0, 0, 255));

    // Style only → medium (3px); color defaults to currentcolor.
    let s = style_with("#t{color:#0a0;border:solid}");
    assert_eq!(s.border, edges(3.0, 3.0, 3.0, 3.0));
    assert_eq!(s.border_color, Color::rgb(0, 0xaa, 0));

    // No style → no border at all (computed width 0).
    assert_eq!(
        style_with("#t{border:1px #000}").border,
        EdgeSizes::default()
    );
    assert_eq!(
        style_with("#t{border:2px solid;border:none}").border,
        EdgeSizes::default()
    );
    assert_eq!(
        style_with("#t{border:2px solid;border:0}").border,
        EdgeSizes::default()
    );

    // Invalid combos are ignored entirely.
    for bad in [
        "1px 2px solid",
        "solid dashed",
        "1px solid red blue",
        "1px wavy red",
        "-1px solid",
    ] {
        let s = style_with(&format!("#t{{border:4px solid #123456; border:{bad}}}"));
        assert_eq!(s.border, edges(4.0, 4.0, 4.0, 4.0), "border: {bad}");
        assert_eq!(
            s.border_color,
            Color::rgb(0x12, 0x34, 0x56),
            "border: {bad}"
        );
    }
}

#[test]
fn border_sides_width_style_color_longhands() {
    let s = style_with("#t{border-bottom:1px solid #ccc}");
    assert_eq!(s.border, edges(0.0, 0.0, 1.0, 0.0));
    assert_eq!(s.border_color, Color::rgb(0xcc, 0xcc, 0xcc));

    let s = style_with("#t{border-width:1px 2px 3px 4px;border-style:solid}");
    assert_eq!(s.border, edges(1.0, 2.0, 3.0, 4.0));

    let s = style_with("#t{border-width:1px 2px 3px 4px;border-style:solid none}");
    assert_eq!(s.border, edges(1.0, 0.0, 3.0, 0.0));

    // `border-width` alone draws nothing (initial style is none).
    assert_eq!(
        style_with("#t{border-width:5px}").border,
        EdgeSizes::default()
    );

    let s = style_with("#t{border:1px solid;border-left-width:6px;border-top-style:none}");
    assert_eq!(s.border, edges(0.0, 1.0, 1.0, 6.0));

    let s = style_with("#t{border:1px solid;border-left:none}");
    assert_eq!(s.border, edges(1.0, 1.0, 1.0, 0.0));

    let s = style_with("#t{border:1px solid;border-color:#111 #222}");
    assert_eq!(s.border_color, Color::rgb(0x11, 0x11, 0x11));
    let s = style_with("#t{color:#333;border:1px solid red;border-color:currentcolor}");
    assert_eq!(s.border_color, Color::rgb(0x33, 0x33, 0x33));

    // Bad counts ignored.
    let s =
        style_with("#t{border:1px solid;border-width:1px 2px 3px 4px 5px;border-style:solid x}");
    assert_eq!(s.border, edges(1.0, 1.0, 1.0, 1.0));
}

#[test]
fn background_shorthand_color_layer() {
    assert_eq!(
        style_with("#t{background:#fc3}").background_color,
        Some(Color::rgb(0xff, 0xcc, 0x33))
    );
    assert_eq!(
        style_with("#t{background:url(a.png) no-repeat center / cover #123}").background_color,
        Some(Color::rgb(0x11, 0x22, 0x33))
    );
    // A shorthand without a color resets it.
    assert_eq!(
        style_with("#t{background-color:red;background:url(a.png) no-repeat}").background_color,
        None
    );
    assert_eq!(
        style_with("#t{background:red;background:none}").background_color,
        None
    );
    assert_eq!(
        style_with("#t{background:red;background:transparent}").background_color,
        None
    );
    // Gradient falls back to the first stop.
    assert_eq!(
        style_with("#t{background:linear-gradient(to right, #0f0, #00f)}").background_color,
        Some(Color::rgb(0, 255, 0))
    );
    // Two colors → invalid → previous value kept.
    assert_eq!(
        style_with("#t{background:#fff;background:red blue}").background_color,
        Some(Color::rgb(255, 255, 255))
    );
}

#[test]
fn font_shorthand_size_weight_family_line_height() {
    let s = style_with("#t{font:bold 20px/1.5 \"Fira Mono\", monospace}");
    assert_eq!(s.font_size, 20.0);
    assert!(s.bold);
    assert!(s.monospace);

    let s = style_with("#t{font-weight:bold;font:italic 2em Georgia, serif}");
    assert_eq!(s.font_size, 32.0);
    assert!(!s.bold, "font shorthand resets weight to normal");
    assert!(!s.monospace);

    let s = style_with("#t{font:700 1rem/24px system-ui}");
    assert!(s.bold);
    assert_eq!(s.font_size, 16.0);

    let s = style_with("#t{font:small-caps 300 large sans-serif}");
    assert!(!s.bold);
    assert_eq!(s.font_size, 18.0);

    // em on other properties uses the shorthand's size.
    let s = style_with("#t{font:10px Arial;padding:2em}");
    assert_eq!(s.padding.top, 20.0);

    // Invalid: no family, no size, garbage line-height, system font — all ignored.
    for bad in [
        "bold 20px",
        "bold Arial",
        "20px/ Arial",
        "20px/x Arial",
        "menu",
        "-3px Arial",
    ] {
        let s = style_with(&format!("#t{{font-size:12px;font-weight:bold;font:{bad}}}"));
        assert_eq!(s.font_size, 12.0, "font: {bad}");
        assert!(s.bold, "font: {bad}");
    }
}

#[test]
fn inset_and_sides_are_stored() {
    let s = style_with("#t{inset:1px 2% auto}");
    assert_eq!(s.inset.top, LengthPct::Px(1.0));
    assert_eq!(s.inset.right, LengthPct::Percent(2.0));
    assert_eq!(s.inset.bottom, LengthPct::Auto);
    assert_eq!(s.inset.left, LengthPct::Percent(2.0));
    let s = style_with("#t{inset:0;left:-1em;top:auto}");
    assert_eq!(s.inset.left, LengthPct::Px(-16.0));
    assert_eq!(s.inset.top, LengthPct::Auto);
    assert_eq!(s.inset.right, LengthPct::Px(0.0));
    let s = style_with("#t{top:3px;top:nope;inset:1px 2px 3px 4px 5px}");
    assert_eq!(s.inset.top, LengthPct::Px(3.0));
}

#[test]
fn length_pct_resolution() {
    assert_eq!(LengthPct::Auto.resolve(Some(100.0)), None);
    assert_eq!(LengthPct::Px(5.0).resolve(None), Some(5.0));
    assert_eq!(LengthPct::Percent(25.0).resolve(Some(200.0)), Some(50.0));
    assert_eq!(LengthPct::Percent(25.0).resolve(None), None);
}

#[test]
fn computed_style_stays_copy_and_compact() {
    fn assert_copy<T: Copy>() {}
    assert_copy::<ComputedStyle>();
    assert_copy::<LengthPct>();
    // 148 bytes before DROP-124; +36 for min/max/inset/box-sizing. Every byte is paid per node
    // twice (style tree + layout tree), so growth needs a reason.
    assert!(std::mem::size_of::<ComputedStyle>() <= 184);
}

#[test]
fn var_components_use_their_fallback_until_custom_properties_land() {
    // Wikipedia's search box: `border: 1px solid var(--border-color-base, #a2a9b1)`.
    let s = style_with("#t{border:1px solid var(--border-color-base,#a2a9b1)}");
    assert_eq!(s.border, edges(1.0, 1.0, 1.0, 1.0));
    assert_eq!(s.border_color, Color::rgb(0xa2, 0xa9, 0xb1));
    // No fallback: the component is dropped, the rest still applies.
    let s = style_with("#t{color:#010203;border:2px solid var(--c)}");
    assert_eq!(s.border, edges(2.0, 2.0, 2.0, 2.0));
    assert_eq!(s.border_color, Color::rgb(1, 2, 3));
    let s = style_with("#t{background:var(--bg,#fff) url(x.png)}");
    assert_eq!(s.background_color, Some(Color::rgb(255, 255, 255)));
    let s = style_with("#t{padding:var(--p,4px) 8px}");
    assert_eq!(s.padding, edges(4.0, 8.0, 4.0, 8.0));
}
