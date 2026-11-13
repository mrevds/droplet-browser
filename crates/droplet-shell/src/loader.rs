use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use droplet_dom::{html, Document, NodeKind};
use droplet_image::DecodedImage;
use droplet_style::{css, Stylesheet};
use droplet_url::Url;

const MAX_STYLESHEETS_PER_PAGE: usize = 32;
const MAX_IMAGES_PER_PAGE: usize = 64;
const MAX_IMAGE_CONCURRENCY: usize = 4;

/// Parsed document, combined stylesheet, interaction flag, and decoded images ready for layout.
pub struct LoadedPage {
    pub final_url: Url,
    pub doc: Document,
    pub stylesheet: Stylesheet,
    pub uses_interaction: bool,
    pub images: HashMap<String, Arc<DecodedImage>>,
    pub title_override: Option<String>,
    pub status_override: String,
}

/// Fetches `url`, normalizes plain-text/empty-error responses into HTML, parses DOM and CSS,
/// and concurrently loads bounded subresource stylesheets and images.
pub fn load_url_document(url: &Url) -> LoadedPage {
    match droplet_net::fetch(url) {
        Ok(resp) => {
            let final_url = resp.url.clone();
            let ct = resp.content_type.to_ascii_lowercase();
            let (html_source, fetch_subresources) =
                if ct.starts_with("text/plain") || ct.starts_with("text/css") {
                    (
                        format!(
                            "<!DOCTYPE html><html><head><title>{}</title></head><body><pre>{}</pre></body></html>",
                            escape_html(final_url.as_str()),
                            escape_html(&resp.body)
                        ),
                        false,
                    )
                } else if resp.status >= 400 && resp.body.trim().is_empty() {
                    (
                        build_error_html(
                            final_url.as_str(),
                            &format!("HTTP status {}", resp.status),
                        ),
                        false,
                    )
                } else {
                    (resp.body, true)
                };

            build_loaded_page(
                final_url,
                &html_source,
                None,
                String::new(),
                fetch_subresources,
            )
        }
        Err(err) => {
            let err_html = build_error_html(url.as_str(), &err.to_string());
            build_loaded_page(
                url.clone(),
                &err_html,
                Some("Load Error".to_string()),
                format!("Load error: {err}"),
                false,
            )
        }
    }
}

/// Builds a `LoadedPage` from raw HTML without performing top-level navigation fetching.
pub fn build_loaded_page(
    final_url: Url,
    html_source: &str,
    title_override: Option<String>,
    status_override: String,
    fetch_subresources: bool,
) -> LoadedPage {
    let doc = html::parse(html_source);

    let style_text = if fetch_subresources {
        collect_combined_css(&doc, &final_url)
    } else {
        doc.collect_style_text()
    };
    let stylesheet = css::parse(&style_text);
    let uses_interaction = droplet_style::stylesheet_uses_interaction(&stylesheet);

    let images = if fetch_subresources {
        load_document_images(&doc, &final_url)
    } else {
        HashMap::new()
    };

    LoadedPage {
        final_url,
        doc,
        stylesheet,
        uses_interaction,
        images,
        title_override,
        status_override,
    }
}

/// Builds a friendly error `LoadedPage` when a URL fails to parse or resolve.
pub fn build_navigation_error_page(base_url: Url, target: &str, err_msg: &str) -> LoadedPage {
    let err_html = build_error_html(target, err_msg);
    build_loaded_page(
        base_url,
        &err_html,
        Some("Navigation Error".to_string()),
        format!("URL error: {err_msg}"),
        false,
    )
}

fn is_data_uri(uri: &str) -> bool {
    uri.as_bytes()
        .get(..5)
        .is_some_and(|head| head.eq_ignore_ascii_case(b"data:"))
}

fn fetch_data_css(uri: &str, base_url: &Url) -> Option<String> {
    let (_, bytes) = droplet_net::fetch_uri_bytes(uri, base_url).ok()?;
    let css_text = String::from_utf8(bytes)
        .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned());
    if css_text.trim().is_empty() {
        None
    } else {
        Some(css_text)
    }
}

/// Collects inline `<style>` and external `<link rel="stylesheet">` rules in document order.
pub fn collect_combined_css(doc: &Document, base_url: &Url) -> String {
    let raw_links = doc.collect_stylesheet_links();
    if raw_links.is_empty() {
        return doc.collect_style_text();
    }

    let mut seen = HashSet::new();
    let mut css_urls = Vec::new();
    let mut has_data_link = false;
    for raw in &raw_links {
        let trimmed = raw.trim();
        if is_data_uri(trimmed) {
            has_data_link = true;
            continue;
        }
        if let Ok(resolved) = base_url.resolve(trimmed) {
            if seen.insert(resolved.as_str().to_string()) {
                css_urls.push(resolved);
                if css_urls.len() >= MAX_STYLESHEETS_PER_PAGE {
                    break;
                }
            }
        }
    }

    if css_urls.is_empty() && !has_data_link {
        return doc.collect_style_text();
    }

    let mut fetched: HashMap<String, String> = HashMap::new();
    if !css_urls.is_empty() {
        let responses = droplet_net::fetch_many(&css_urls);
        for (url, res) in css_urls.into_iter().zip(responses) {
            if let Ok(resp) = res {
                if resp.status < 400 && !resp.body.trim().is_empty() {
                    fetched.insert(url.as_str().to_string(), resp.body);
                }
            }
        }
    }

    if fetched.is_empty() && !has_data_link {
        return doc.collect_style_text();
    }

    let mut parts = Vec::new();
    let mut data_fetched = 0usize;
    let mut stack = vec![doc.root()];
    while let Some(curr) = stack.pop() {
        let Some(node) = doc.get(curr) else {
            continue;
        };
        if let NodeKind::Element(data) = &node.kind {
            if data.tag_name.eq_ignore_ascii_case("style") {
                let css = doc.text_content(curr);
                if !css.trim().is_empty() {
                    parts.push(css);
                }
                continue;
            }
            if data.tag_name.eq_ignore_ascii_case("link") {
                let is_stylesheet = data.attr("rel").is_some_and(|rel| {
                    rel.split_ascii_whitespace()
                        .any(|t| t.eq_ignore_ascii_case("stylesheet"))
                });
                if is_stylesheet {
                    if let Some(href) = data.attr("href") {
                        let trimmed = href.trim();
                        if !trimmed.is_empty() {
                            if is_data_uri(trimmed) {
                                if data_fetched < MAX_STYLESHEETS_PER_PAGE {
                                    data_fetched += 1;
                                    if let Some(css) = fetch_data_css(trimmed, base_url) {
                                        parts.push(css);
                                    }
                                }
                            } else if let Ok(resolved) = base_url.resolve(trimmed) {
                                if let Some(css) = fetched.get(resolved.as_str()) {
                                    parts.push(css.clone());
                                }
                            }
                        }
                    }
                }
            }
        }
        for &child in node.children.iter().rev() {
            stack.push(child);
        }
    }

    parts.join("\n")
}

fn collect_image_sources(doc: &Document) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut sources = Vec::new();
    let mut stack = vec![doc.root()];
    while let Some(curr) = stack.pop() {
        let Some(node) = doc.get(curr) else {
            continue;
        };
        if let NodeKind::Element(data) = &node.kind {
            if data.tag_name.eq_ignore_ascii_case("img") {
                if let Some(src) = data.attr("src") {
                    let trimmed = src.trim();
                    if !trimmed.is_empty() && seen.insert(trimmed.to_string()) {
                        sources.push(trimmed.to_string());
                        if sources.len() >= MAX_IMAGES_PER_PAGE {
                            break;
                        }
                    }
                }
            }
        }
        for &child in node.children.iter().rev() {
            stack.push(child);
        }
    }
    sources
}

fn fetch_and_decode_image(src: &str, base_url: &Url) -> Option<Arc<DecodedImage>> {
    let (_content_type, bytes) = droplet_net::fetch_uri_bytes(src, base_url).ok()?;
    let decoded = droplet_image::decode(&bytes).ok()?;
    Some(Arc::new(decoded))
}

/// Collects unique `<img src>` URIs (capped at `MAX_IMAGES_PER_PAGE`) and fetches/decodes them
/// concurrently across a bounded worker pool.
pub fn load_document_images(doc: &Document, base_url: &Url) -> HashMap<String, Arc<DecodedImage>> {
    let sources = collect_image_sources(doc);
    if sources.is_empty() {
        return HashMap::new();
    }
    if sources.len() == 1 {
        let mut out = HashMap::with_capacity(1);
        if let Some(decoded) = fetch_and_decode_image(&sources[0], base_url) {
            out.insert(sources[0].clone(), decoded);
        }
        return out;
    }

    let worker_count = sources.len().min(MAX_IMAGE_CONCURRENCY);
    let next = AtomicUsize::new(0);
    let batches: Vec<Vec<(String, Arc<DecodedImage>)>> = std::thread::scope(|scope| {
        let mut handles = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            handles.push(scope.spawn(|| {
                let mut local = Vec::new();
                loop {
                    let idx = next.fetch_add(1, Ordering::Relaxed);
                    if idx >= sources.len() {
                        break;
                    }
                    let src = &sources[idx];
                    if let Some(decoded) = fetch_and_decode_image(src, base_url) {
                        local.push((src.clone(), decoded));
                    }
                }
                local
            }));
        }
        handles
            .into_iter()
            .filter_map(|h| h.join().ok())
            .collect()
    });

    let mut out = HashMap::with_capacity(sources.len());
    for batch in batches {
        for (src, img) in batch {
            out.insert(src, img);
        }
    }
    out
}

/// Escapes special HTML characters for safe interpolation into generated HTML pages.
pub fn escape_html(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Generates Droplet's built-in error HTML page for navigation or load failures.
pub fn build_error_html(target: &str, message: &str) -> String {
    let esc_target = escape_html(target);
    let esc_msg = escape_html(message);
    format!(
        r#"<!DOCTYPE html>
<html>
<head>
<title>Droplet - Navigation Error</title>
<style>
body {{ font-size: 16px; color: #0f172a; background-color: #f8fafc; padding: 24px; }}
.card {{ background-color: #ffffff; border: 1px solid #cbd5e1; padding: 20px; max-width: 720px; }}
h1 {{ color: #b91c1c; margin-bottom: 12px; }}
.target {{ background-color: #f1f5f9; padding: 8px; border: 1px solid #e2e8f0; margin: 10px 0; }}
</style>
</head>
<body>
<div class="card">
  <h1>Unable to open page</h1>
  <div class="target">Target: {esc_target}</div>
  <p>{esc_msg}</p>
  <p><a href="about:droplet">Return to about:droplet</a> | <a href="about:test">Open about:test</a></p>
</div>
</body>
</html>"#
    )
}
