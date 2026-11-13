//! Per-stage pipeline benchmark (`droplet --bench`).
//!
//! The network/subresource stage runs once; the CPU stages (HTML parse, CSS parse,
//! style, layout, display list, raster) run `runs` times on the same input and the
//! median is reported, so numbers are comparable between commits.

use std::time::Instant;

use droplet_dom::html;
use droplet_font::FontEngine;
use droplet_gfx::{Canvas, Color};
use droplet_layout::LayoutBox;
use droplet_paint::{build_display_list, render_frame, ChromeState};
use droplet_style::{css, StyleContext};
use droplet_url::Url;

use crate::loader;

/// Median wall-clock milliseconds per pipeline stage plus tree sizes.
#[derive(Debug, Clone, PartialEq)]
pub struct BenchReport {
    pub url: String,
    pub runs: usize,
    pub html_bytes: usize,
    pub css_bytes: usize,
    pub dom_nodes: usize,
    pub layout_boxes: usize,
    pub display_items: usize,
    /// Top-level fetch plus external CSS and image fetch/decode (single run).
    pub fetch_ms: f64,
    pub html_ms: f64,
    pub css_ms: f64,
    pub style_ms: f64,
    pub layout_ms: f64,
    pub display_list_ms: f64,
    pub raster_ms: f64,
}

impl BenchReport {
    /// Sum of the repeated CPU stages (everything except the network stage).
    pub fn cpu_ms(&self) -> f64 {
        self.html_ms + self.css_ms + self.style_ms + self.layout_ms + self.display_list_ms + self.raster_ms
    }

    /// One `key=value` line, stable for scripts and diffing between commits.
    pub fn to_line(&self) -> String {
        format!(
            "url={} runs={} html_bytes={} css_bytes={} dom_nodes={} layout_boxes={} display_items={} \
             fetch_ms={:.2} html_ms={:.3} css_ms={:.3} style_ms={:.3} layout_ms={:.3} \
             display_list_ms={:.3} raster_ms={:.3} cpu_ms={:.3}",
            self.url,
            self.runs,
            self.html_bytes,
            self.css_bytes,
            self.dom_nodes,
            self.layout_boxes,
            self.display_items,
            self.fetch_ms,
            self.html_ms,
            self.css_ms,
            self.style_ms,
            self.layout_ms,
            self.display_list_ms,
            self.raster_ms,
            self.cpu_ms()
        )
    }
}

fn median(samples: &mut [f64]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    samples.sort_by(|a, b| a.total_cmp(b));
    samples[samples.len() / 2]
}

fn ms_since(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

fn count_boxes(root: &LayoutBox) -> usize {
    let mut count = 0;
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        count += 1;
        stack.extend(node.children.iter());
    }
    count
}

/// Benchmarks the full pipeline for `url` at `width`x`height`, `runs` times (min 1).
pub fn bench_url(url: &Url, runs: usize, width: usize, height: usize) -> Result<BenchReport, String> {
    let runs = runs.max(1);

    let fetch_start = Instant::now();
    let resp = droplet_net::fetch(url).map_err(|e| format!("fetch '{}': {e}", url.as_str()))?;
    let base_url = resp.url.clone();
    let body = resp.body;
    let first_doc = html::parse(&body);
    let style_text = loader::collect_combined_css(&first_doc, &base_url);
    let images = loader::load_document_images(&first_doc, &base_url);
    let fetch_ms = ms_since(fetch_start);

    let ctx = StyleContext {
        viewport_width: width as f32,
        viewport_height: height as f32,
        ..StyleContext::default()
    };
    let mut font = FontEngine::new();
    let mut canvas = Canvas::new(width.max(1), height.max(1), Color::rgb(255, 255, 255));
    let url_text = base_url.as_str().to_string();
    let titles = ["bench"];

    let mut t_html = Vec::with_capacity(runs);
    let mut t_css = Vec::with_capacity(runs);
    let mut t_style = Vec::with_capacity(runs);
    let mut t_layout = Vec::with_capacity(runs);
    let mut t_dl = Vec::with_capacity(runs);
    let mut t_raster = Vec::with_capacity(runs);
    let (mut dom_nodes, mut layout_boxes, mut display_items) = (0, 0, 0);

    for _ in 0..runs {
        let start = Instant::now();
        let doc = html::parse(&body);
        t_html.push(ms_since(start));
        dom_nodes = doc.nodes.len();

        let start = Instant::now();
        let sheet = css::parse(&style_text);
        t_css.push(ms_since(start));

        let start = Instant::now();
        let styled = droplet_style::style_tree_with_context(&doc, &sheet, &ctx)
            .ok_or_else(|| "empty style tree".to_string())?;
        t_style.push(ms_since(start));

        let start = Instant::now();
        let root = droplet_layout::layout_tree(&doc, &styled, ctx.viewport_width, ctx.viewport_height, &images);
        t_layout.push(ms_since(start));
        layout_boxes = count_boxes(&root);

        let start = Instant::now();
        let list = build_display_list(&root);
        t_dl.push(ms_since(start));
        display_items = list.len();

        let chrome = ChromeState {
            url_input: &url_text,
            url_focused: false,
            autocomplete_suffix: "",
            can_back: false,
            can_forward: false,
            status_text: "",
            timing_badge: "",
            tab_titles: &titles,
            active_tab: 0,
            scroll_y: 0.0,
            content_height: 0.0,
        };
        let start = Instant::now();
        render_frame(&mut canvas, &mut font, &list, &chrome);
        t_raster.push(ms_since(start));
    }

    Ok(BenchReport {
        url: base_url.as_str().to_string(),
        runs,
        html_bytes: body.len(),
        css_bytes: style_text.len(),
        dom_nodes,
        layout_boxes,
        display_items,
        fetch_ms,
        html_ms: median(&mut t_html),
        css_ms: median(&mut t_css),
        style_ms: median(&mut t_style),
        layout_ms: median(&mut t_layout),
        display_list_ms: median(&mut t_dl),
        raster_ms: median(&mut t_raster),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn median_picks_middle_sample() {
        assert_eq!(median(&mut []), 0.0);
        assert_eq!(median(&mut [5.0, 1.0, 3.0]), 3.0);
    }

    #[test]
    fn bench_reports_stages_for_local_cyrillic_page() {
        let dir = std::env::temp_dir().join(format!("droplet_bench_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let page = dir.join("bench.html");
        std::fs::write(
            &page,
            "<!DOCTYPE html><html><head><style>p { color: red; margin: 4px }</style></head>\
             <body><h1>Скорость</h1><p>Привет, мир</p><p>Ещё абзац</p></body></html>",
        )
        .unwrap();
        let url = Url::parse(&format!("file://{}", page.display())).unwrap();
        let report = bench_url(&url, 3, 320, 240).unwrap();
        assert_eq!(report.runs, 3);
        assert!(report.dom_nodes > 5);
        assert!(report.layout_boxes > 3);
        assert!(report.display_items > 0);
        assert!(report.css_bytes > 0);
        let line = report.to_line();
        assert!(line.contains("cpu_ms="));
        assert!(line.starts_with("url=file://"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
