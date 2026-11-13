use droplet_gfx::Color;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Display {
    Block,
    Inline,
    InlineBlock,
    Flex,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FlexDirection {
    #[default]
    Row,
    Column,
    RowReverse,
    ColumnReverse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FlexWrap {
    #[default]
    NoWrap,
    Wrap,
    WrapReverse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum JustifyContent {
    #[default]
    FlexStart,
    FlexEnd,
    Center,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AlignItems {
    #[default]
    Stretch,
    FlexStart,
    FlexEnd,
    Center,
    Baseline,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum FlexBasis {
    #[default]
    Auto,
    Px(f32),
    Percent(f32),
}

/// `box-sizing`: which box `width`/`height` and their `min-`/`max-` limits measure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BoxSizing {
    #[default]
    ContentBox,
    BorderBox,
}

/// A size that is a length, a percentage of the containing block, or absent
/// (`auto` for `min-*`/insets, `none` for `max-*`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum LengthPct {
    #[default]
    Auto,
    Px(f32),
    Percent(f32),
}

impl LengthPct {
    /// Resolves against `basis`; a percentage with an unknown basis is `None`.
    pub fn resolve(self, basis: Option<f32>) -> Option<f32> {
        match self {
            LengthPct::Auto => None,
            LengthPct::Px(px) => Some(px),
            LengthPct::Percent(pct) => basis.map(|b| b * (pct / 100.0)),
        }
    }
}

/// `top`/`right`/`bottom`/`left` (and the `inset` shorthand); used by positioned layout.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct InsetSizes {
    pub top: LengthPct,
    pub right: LengthPct,
    pub bottom: LengthPct,
    pub left: LengthPct,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct EdgeSizes {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ComputedStyle {
    pub display: Display,
    pub color: Color,
    pub background_color: Option<Color>,
    pub border_color: Color,
    pub margin: EdgeSizes,
    pub padding: EdgeSizes,
    pub border: EdgeSizes,
    pub border_radius: f32,
    pub width: LengthPct,
    pub max_width: LengthPct,
    pub min_width: LengthPct,
    pub height: LengthPct,
    pub min_height: LengthPct,
    pub max_height: LengthPct,
    pub box_sizing: BoxSizing,
    pub inset: InsetSizes,
    pub font_size: f32,
    pub bold: bool,
    pub monospace: bool,
    pub text_underline: bool,
    pub margin_left_auto: bool,
    pub margin_right_auto: bool,
    pub preformatted: bool,
    pub flex_direction: FlexDirection,
    pub flex_wrap: FlexWrap,
    pub justify_content: JustifyContent,
    pub align_items: AlignItems,
    pub gap: f32,
    pub flex_grow: f32,
    pub flex_shrink: f32,
    pub flex_basis: FlexBasis,
}

impl Default for ComputedStyle {
    fn default() -> Self {
        let default_color = Color::rgb(15, 23, 42);
        Self {
            display: Display::Inline,
            color: default_color,
            background_color: None,
            border_color: default_color,
            margin: EdgeSizes::default(),
            padding: EdgeSizes::default(),
            border: EdgeSizes::default(),
            border_radius: 0.0,
            width: LengthPct::Auto,
            max_width: LengthPct::Auto,
            min_width: LengthPct::Auto,
            height: LengthPct::Auto,
            min_height: LengthPct::Auto,
            max_height: LengthPct::Auto,
            box_sizing: BoxSizing::ContentBox,
            inset: InsetSizes::default(),
            font_size: 16.0,
            bold: false,
            monospace: false,
            text_underline: false,
            margin_left_auto: false,
            margin_right_auto: false,
            preformatted: false,
            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::NoWrap,
            justify_content: JustifyContent::FlexStart,
            align_items: AlignItems::Stretch,
            gap: 0.0,
            flex_grow: 0.0,
            flex_shrink: 1.0,
            flex_basis: FlexBasis::Auto,
        }
    }
}
