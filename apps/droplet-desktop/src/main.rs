use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process;
use std::sync::Arc;

use droplet_dom::html;
use droplet_dom::{Document, ElementData, NodeKind};
use droplet_image::DecodedImage;
use droplet_shell::Browser;
use droplet_style::css::{self, Stylesheet};
use droplet_style::StyleContext;
use droplet_url::Url;

const DEFAULT_URL: &str = "about:droplet";
const HEADLESS_VIEWPORT_WIDTH: usize = 1024;
const HEADLESS_VIEWPORT_HEIGHT: usize = 768;

fn print_usage() {
    println!(
        "Droplet — lightweight graphical web browser engine\n\n\
         USAGE:\n\
         \tdroplet [URL]\n\
         \tdroplet --screenshot <out.ppm> [URL]\n\
         \tdroplet --dump-dom [URL]\n\
         \tdroplet --dump-layout [URL]\n\
         \tdroplet --help | -h\n\n\
         BUILT-IN PAGES:\n\
         \tabout:droplet, about:roadmap, about:architecture, about:test, about:blank"
    );
}

fn load_document(url_str: &str) -> Result<(Url, Document), String> {
    let url = Url::parse(url_str).map_err(|e| format!("Invalid URL '{url_str}': {e}"))?;
    let resp =
        droplet_net::fetch(&url).map_err(|e| format!("Failed to fetch '{}': {e}", url.as_str()))?;
    Ok((resp.url, html::parse(&resp.body)))
}

enum StyleChunk {
    Inline(String),
    External(usize),
}

fn is_data_uri(uri: &str) -> bool {
    uri.as_bytes()
        .get(..5)
        .is_some_and(|head| head.eq_ignore_ascii_case(b"data:"))
}

fn stylesheet_link_href(data: &ElementData) -> Option<&str> {
    if !data.tag_name.eq_ignore_ascii_case("link") {
        return None;
    }
    let is_stylesheet = data.attr("rel").is_some_and(|rel| {
        rel.split_ascii_whitespace()
            .any(|token| token.eq_ignore_ascii_case("stylesheet"))
    });
    if !is_stylesheet {
        return None;
    }
    let href = data.attr("href")?.trim();
    if href.is_empty() {
        None
    } else {
        Some(href)
    }
}

fn img_src(data: &ElementData) -> Option<&str> {
    if !data.tag_name.eq_ignore_ascii_case("img") {
        return None;
    }
    let src = data.attr("src")?.trim();
    if src.is_empty() {
        None
    } else {
        Some(src)
    }
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

fn load_document_stylesheet(doc: &Document, base_url: &Url) -> Stylesheet {
    let mut chunks = Vec::new();
    let mut fetch_urls = Vec::new();
    let mut url_indices: HashMap<String, usize> = HashMap::new();
    let mut stack = vec![doc.root()];

    while let Some(curr) = stack.pop() {
        let Some(node) = doc.get(curr) else {
            continue;
        };
        if let NodeKind::Element(data) = &node.kind {
            if data.tag_name.eq_ignore_ascii_case("style") {
                let css_text = doc.text_content(curr);
                if !css_text.trim().is_empty() {
                    chunks.push(StyleChunk::Inline(css_text));
                }
                continue;
            }
            if let Some(href) = stylesheet_link_href(data) {
                if is_data_uri(href) {
                    if let Some(css_text) = fetch_data_css(href, base_url) {
                        chunks.push(StyleChunk::Inline(css_text));
                    }
                } else if let Ok(resolved) = base_url.resolve(href) {
                    let idx = if let Some(&existing) = url_indices.get(resolved.as_str()) {
                        existing
                    } else {
                        let idx = fetch_urls.len();
                        url_indices.insert(resolved.as_str().to_string(), idx);
                        fetch_urls.push(resolved);
                        idx
                    };
                    chunks.push(StyleChunk::External(idx));
                }
            }
        }
        for &child in node.children.iter().rev() {
            stack.push(child);
        }
    }

    let fetched = droplet_net::fetch_many(&fetch_urls);
    let parsed_external: Vec<Option<Stylesheet>> = fetched
        .into_iter()
        .map(|res| match res {
            Ok(resp) if resp.status < 400 => Some(css::parse(&resp.body)),
            _ => None,
        })
        .collect();

    let mut rules = Vec::new();
    for chunk in chunks {
        match chunk {
            StyleChunk::Inline(css_text) => {
                rules.extend(css::parse(&css_text).rules);
            }
            StyleChunk::External(idx) => {
                if let Some(Some(sheet)) = parsed_external.get(idx) {
                    rules.extend(sheet.rules.iter().cloned());
                }
            }
        }
    }

    Stylesheet { rules }
}

fn fetch_decoded_image(src: &str, base_url: &Url) -> Option<DecodedImage> {
    let (_, bytes) = droplet_net::fetch_uri_bytes(src, base_url).ok()?;
    droplet_image::decode(&bytes).ok()
}

fn load_document_images(doc: &Document, base_url: &Url) -> HashMap<String, Arc<DecodedImage>> {
    let mut images = HashMap::new();
    let mut seen = HashSet::new();
    let mut stack = vec![doc.root()];

    while let Some(curr) = stack.pop() {
        let Some(node) = doc.get(curr) else {
            continue;
        };
        if let NodeKind::Element(data) = &node.kind {
            if let Some(src) = img_src(data) {
                if seen.insert(src.to_string()) {
                    if let Some(decoded) = fetch_decoded_image(src, base_url) {
                        images.insert(src.to_string(), Arc::new(decoded));
                    }
                }
            }
        }
        for &child in node.children.iter().rev() {
            stack.push(child);
        }
    }

    images
}

fn dump_layout_for_url(url: &str) -> Result<String, String> {
    let (base_url, doc) = load_document(url)?;
    let sheet = load_document_stylesheet(&doc, &base_url);
    let images = load_document_images(&doc, &base_url);
    let vp_w = HEADLESS_VIEWPORT_WIDTH as f32;
    let vp_h = HEADLESS_VIEWPORT_HEIGHT as f32;
    let ctx = StyleContext {
        viewport_width: vp_w,
        viewport_height: vp_h,
        hovered_node: None,
        active_node: None,
    };
    if let Some(styled) = droplet_style::style_tree_with_context(&doc, &sheet, &ctx) {
        let root = droplet_layout::layout_tree(&doc, &styled, vp_w, vp_h, &images);
        Ok(droplet_layout::dump_layout(&root))
    } else {
        Ok("(empty layout tree)\n".to_string())
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.is_empty() {
        run_gui(DEFAULT_URL);
        return;
    }

    match args[0].as_str() {
        "--help" | "-h" => {
            print_usage();
        }
        "--dump-dom" => {
            if args.len() > 2 {
                eprintln!("Error: unexpected extra arguments for --dump-dom");
                print_usage();
                process::exit(2);
            }
            let url = args.get(1).map(String::as_str).unwrap_or(DEFAULT_URL);
            match load_document(url) {
                Ok((_, doc)) => print!("{}", doc.dump_tree()),
                Err(err) => {
                    eprintln!("Error: {err}");
                    process::exit(1);
                }
            }
        }
        "--dump-layout" => {
            if args.len() > 2 {
                eprintln!("Error: unexpected extra arguments for --dump-layout");
                print_usage();
                process::exit(2);
            }
            let url = args.get(1).map(String::as_str).unwrap_or(DEFAULT_URL);
            match dump_layout_for_url(url) {
                Ok(dumped) => print!("{dumped}"),
                Err(err) => {
                    eprintln!("Error: {err}");
                    process::exit(1);
                }
            }
        }
        "--screenshot" => {
            if args.len() < 2 || args.len() > 3 {
                eprintln!("Error: --screenshot requires <out.ppm> and optional [URL]");
                print_usage();
                process::exit(2);
            }
            let out_path = Path::new(&args[1]);
            let url = args.get(2).map(String::as_str).unwrap_or(DEFAULT_URL);
            let mut browser =
                Browser::new(url, HEADLESS_VIEWPORT_WIDTH, HEADLESS_VIEWPORT_HEIGHT);
            let canvas = browser.render();
            if let Err(err) = canvas.write_ppm(out_path) {
                eprintln!(
                    "Error: failed to write screenshot '{}': {err}",
                    out_path.display()
                );
                process::exit(1);
            }
            println!(
                "Saved {}x{} screenshot to {}",
                canvas.width,
                canvas.height,
                out_path.display()
            );
        }
        flag if flag.starts_with('-') => {
            eprintln!("Error: unknown option '{flag}'");
            print_usage();
            process::exit(2);
        }
        url => {
            if args.len() > 1 {
                eprintln!("Error: unexpected extra arguments");
                print_usage();
                process::exit(2);
            }
            run_gui(url);
        }
    }
}

fn run_gui(url: &str) {
    let browser = Browser::new(url, 1140, 780);
    if let Err(err) = droplet_platform::run(browser) {
        eprintln!("Could not start graphical window: {err}");
        eprintln!("Tip: if no display server is attached, you can run Droplet headlessly using:");
        eprintln!("  droplet --screenshot out.ppm {url}");
        eprintln!("  droplet --dump-dom {url}");
        eprintln!("  droplet --dump-layout {url}");
        process::exit(1);
    }
}
