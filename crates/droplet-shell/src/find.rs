//! Find-in-page state and UTF-8-safe case-insensitive matching.
//!
//! Folding only maps characters whose UTF-8 byte length is unchanged (ASCII
//! A–Z and Russian Cyrillic А–Я/Ё), so byte offsets found in the folded text
//! are always valid slice boundaries in the original text.

use droplet_dom::NodeId;
use droplet_gfx::font;
use droplet_gfx::Rect;
use droplet_layout::{BoxType, LayoutBox};

/// One visible match in page coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FindMatch {
    pub rect: Rect,
    pub node_id: Option<NodeId>,
}

/// Find bar query, ordered matches and active index.
#[derive(Debug, Clone, Default)]
pub struct FindState {
    pub open: bool,
    pub query: String,
    pub matches: Vec<FindMatch>,
    pub active: usize,
    pub dirty: bool,
}

impl FindState {
    pub fn show(&mut self) {
        self.open = true;
        self.dirty = true;
    }

    pub fn hide(&mut self) {
        self.open = false;
        self.matches.clear();
        self.active = 0;
        self.dirty = false;
    }

    pub fn set_query(&mut self, query: impl Into<String>) {
        self.query = query.into();
        self.active = 0;
        self.dirty = true;
    }

    pub fn push_char(&mut self, ch: char) {
        self.query.push(ch);
        self.active = 0;
        self.dirty = true;
    }

    pub fn pop_char(&mut self) {
        self.query.pop();
        self.active = 0;
        self.dirty = true;
    }

    pub fn recompute(&mut self, root: Option<&LayoutBox>) {
        self.matches = match root {
            Some(root) if !self.query.trim().is_empty() => {
                collect_find_matches(root, &self.query)
            }
            _ => Vec::new(),
        };
        if self.active >= self.matches.len() {
            self.active = 0;
        }
        self.dirty = false;
    }

    /// Advances to the next/previous match with wrap-around.
    pub fn step(&mut self, forward: bool) -> bool {
        if self.matches.is_empty() {
            return false;
        }
        let len = self.matches.len();
        self.active = if forward {
            (self.active + 1) % len
        } else {
            (self.active + len - 1) % len
        };
        true
    }

    /// One-based current index for display; zero when there are no matches.
    pub fn current(&self) -> usize {
        if self.matches.is_empty() {
            0
        } else {
            self.active + 1
        }
    }

    pub fn total(&self) -> usize {
        self.matches.len()
    }

    pub fn active_rect(&self) -> Option<Rect> {
        self.matches.get(self.active).map(|item| item.rect)
    }
}

/// Folds ASCII A–Z and Russian Cyrillic А–Я/Ё to lowercase.
pub fn fold_char(ch: char) -> char {
    match ch {
        'A'..='Z' => ((ch as u8) + 32) as char,
        '\u{0410}'..='\u{042F}' => char::from_u32(ch as u32 + 32).unwrap_or(ch),
        '\u{0401}' => '\u{0451}',
        _ => ch,
    }
}

/// Lowercases text without changing the UTF-8 byte length of any mapped char.
pub fn fold_text(input: &str) -> String {
    input.chars().map(fold_char).collect()
}

/// Returns non-overlapping `(start, end)` byte ranges of `query` in `line`.
pub fn find_ranges(line: &str, query: &str) -> Vec<(usize, usize)> {
    let folded = fold_text(line);
    let needle = fold_text(query);
    find_ranges_folded(&folded, &needle)
        .into_iter()
        .filter(|(start, end)| line.is_char_boundary(*start) && line.is_char_boundary(*end))
        .collect()
}

fn find_ranges_folded(folded_line: &str, folded_query: &str) -> Vec<(usize, usize)> {
    if folded_query.is_empty() {
        return Vec::new();
    }
    folded_line
        .match_indices(folded_query)
        .map(|(start, matched)| (start, start + matched.len()))
        .collect()
}

/// Walks the layout tree in document order and collects every visible match.
pub fn collect_find_matches(root: &LayoutBox, query: &str) -> Vec<FindMatch> {
    let folded_query = fold_text(query.trim());
    if folded_query.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if let BoxType::Text {
            lines,
            line_height,
            font_size,
            monospace,
            ..
        } = &node.box_type
        {
            let y_pad = ((*line_height - *font_size) * 0.5).max(0.0);
            for (line_idx, line) in lines.iter().enumerate() {
                let folded = fold_text(line);
                for (start, end) in find_ranges_folded(&folded, &folded_query) {
                    if !line.is_char_boundary(start) || !line.is_char_boundary(end) {
                        continue;
                    }
                    let prefix_w = font::measure_text_px(
                        &line[..start],
                        *font_size,
                        node.style.bold,
                        *monospace,
                    );
                    let match_w = font::measure_text_px(
                        &line[start..end],
                        *font_size,
                        node.style.bold,
                        *monospace,
                    );
                    out.push(FindMatch {
                        node_id: node.node_id,
                        rect: Rect {
                            x: node.dimensions.content.x + prefix_w,
                            y: node.dimensions.content.y
                                + (line_idx as f32) * *line_height
                                + y_pad,
                            width: match_w.max(1.0),
                            height: *line_height,
                        },
                    });
                }
            }
        }
        for child in node.children.iter().rev() {
            stack.push(child);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use droplet_layout::{Dimensions, LayoutBox};
    use droplet_style::ComputedStyle;

    fn text_box(lines: &[&str], x: f32, y: f32) -> LayoutBox {
        LayoutBox {
            node_id: Some(7),
            dimensions: Dimensions {
                content: Rect {
                    x,
                    y,
                    width: 200.0,
                    height: 40.0,
                },
                ..Dimensions::default()
            },
            box_type: BoxType::Text {
                lines: lines.iter().map(|line| line.to_string()).collect(),
                line_height: 20.0,
                font_size: 16.0,
                font_scale: 1,
                monospace: false,
            },
            style: ComputedStyle::default(),
            href: None,
            children: Vec::new(),
        }
    }

    #[test]
    fn folding_handles_ascii_and_cyrillic_but_keeps_lengths() {
        assert_eq!(fold_text("HELLO"), "hello");
        assert_eq!(fold_text("ПРИВЕТ ЁЖ"), "привет ёж");
        for text in ["Hello", "ПРИВЕТ", "Ёж", "😀"] {
            assert_eq!(fold_text(text).len(), text.len());
        }
    }

    #[test]
    fn ranges_are_utf8_safe_and_case_insensitive() {
        let line = "Привет, мир! ПРИВЕТ!";
        let ranges = find_ranges(line, "привет");
        assert_eq!(ranges.len(), 2);
        assert_eq!(&line[ranges[0].0..ranges[0].1], "Привет");
        assert_eq!(&line[ranges[1].0..ranges[1].1], "ПРИВЕТ");

        let mixed = "Droplet DROPLET droplet";
        assert_eq!(find_ranges(mixed, "Droplet").len(), 3);
        assert!(find_ranges("hello", "").is_empty());
        assert!(find_ranges("Привет", "нет такого").is_empty());
    }

    #[test]
    fn collect_matches_returns_geometry_for_every_line() {
        let root = text_box(&["Hello World", "hello"], 10.0, 30.0);
        let matches = collect_find_matches(&root, "HELLO");
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].node_id, Some(7));
        assert!((matches[0].rect.x - 10.0).abs() < 0.01);
        assert!((matches[0].rect.y - 32.0).abs() < 0.01);
        assert!(matches[0].rect.width > 1.0);
        assert!((matches[1].rect.y - 52.0).abs() < 0.01);
        assert!((matches[1].rect.x - 10.0).abs() < 0.01);
    }

    #[test]
    fn next_previous_wrap_and_indicator() {
        let mut state = FindState::default();
        state.show();
        state.set_query("x");
        state.matches = vec![
            FindMatch {
                rect: Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 1.0,
                    height: 1.0,
                },
                node_id: None,
            };
            3
        ];
        assert_eq!(state.current(), 1);
        assert_eq!(state.current(), 1);
        assert!(state.step(true));
        assert_eq!(state.current(), 2);
        assert!(state.step(true));
        assert_eq!(state.current(), 3);
        assert!(state.step(true));
        assert_eq!(state.current(), 1);
        assert!(state.step(false));
        assert_eq!(state.current(), 3);
        state.matches.clear();
        assert!(!state.step(true));
        assert_eq!(state.current(), 0);
    }
}
