//! Zero-network, zero-filesystem renderer entry point for sandboxed child execution (`DROP-010`).

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use droplet_dom::{html, Document, NodeId, NodeKind};
use droplet_font::FontEngine;
use droplet_gfx::{Canvas, Color, DamageRect};
use droplet_image::DecodedImage;
use droplet_js::{DomEventInput, JsRuntime};
use droplet_layout::{ControlKind, LayoutBox};
use droplet_paint::{
    build_display_list_with_forms, chrome_damage_rect, render_frame_damaged, render_frame_ext,
    viewport_damage_rect, ChromeState, DisplayCommand, FormVisualState, CHROME_HEIGHT,
    STATUS_BAR_HEIGHT,
};
use droplet_style::{css, stylesheet_uses_interaction, StyleContext, Stylesheet};
use droplet_url::Url;

use crate::forms::{self, FormState};
use crate::js::{ScriptUnit, ShellDomHost, TabJsState, MAX_JS_LOG_ENTRIES, MAX_SCRIPTS_PER_PAGE};
use crate::tab::DirtyFlags;
use crate::{compute_content_height, compute_layout_with_ctx};

/// Pre-fetched page bundle handed to [`HeadlessRenderer::load_payload`].
pub struct RenderPayload<'a> {
    pub url: &'a str,
    pub html: &'a str,
    pub assets: &'a [(String, Vec<u8>)],
    pub viewport_width: usize,
    pub viewport_height: usize,
    pub scroll_y: f32,
}

/// Frame diagnostics and metadata produced by [`HeadlessRenderer::paint_into_slice`].
#[derive(Debug, Clone, PartialEq)]
pub struct RenderedFrameInfo {
    pub title: String,
    pub hover_href: Option<String>,
    pub damage: DamageRect,
    pub layout_ms: f32,
    pub paint_ms: f32,
    pub console_logs: Vec<String>,
}

/// Self-contained in-memory renderer performing no network or filesystem I/O.
pub struct HeadlessRenderer {
    viewport_width: usize,
    viewport_height: usize,
    font_engine: FontEngine,
    url: String,
    title: String,
    doc: Option<Document>,
    stylesheet: Option<Stylesheet>,
    images: HashMap<String, Arc<DecodedImage>>,
    js: TabJsState,
    forms: FormState,
    dirty: DirtyFlags,
    layout_root: Option<LayoutBox>,
    display_list: Option<Vec<DisplayCommand>>,
    uses_interaction: bool,
    scroll_y: f32,
    content_height: f32,
    hovered_node: Option<NodeId>,
    active_node: Option<NodeId>,
    pressed_node: Option<NodeId>,
    hover_href: Option<String>,
    pending_damage: DamageRect,
    last_layout_ms: f32,
    has_painted_frame: bool,
}

impl HeadlessRenderer {
    /// Construct a fresh renderer with the given viewport pixel dimensions.
    pub fn new(viewport_w: usize, viewport_h: usize) -> Self {
        let w = viewport_w.max(1);
        let h = viewport_h.max(1);
        Self {
            viewport_width: w,
            viewport_height: h,
            font_engine: FontEngine::new(),
            url: String::new(),
            title: "Droplet".to_string(),
            doc: None,
            stylesheet: None,
            images: HashMap::new(),
            js: TabJsState::default(),
            forms: FormState::default(),
            dirty: DirtyFlags::default(),
            layout_root: None,
            display_list: None,
            uses_interaction: false,
            scroll_y: 0.0,
            content_height: 0.0,
            hovered_node: None,
            active_node: None,
            pressed_node: None,
            hover_href: None,
            pending_damage: DamageRect::full(w, h),
            last_layout_ms: 0.0,
            has_painted_frame: false,
        }
    }

    /// Load a pre-fetched HTML document and its resolved assets into the renderer.
    pub fn load_payload(&mut self, payload: RenderPayload<'_>) {
        self.viewport_width = payload.viewport_width.max(1);
        self.viewport_height = payload.viewport_height.max(1);
        self.url = payload.url.to_string();

        let base_url = Url::parse(payload.url)
            .unwrap_or_else(|_| Url::parse("about:blank").expect("about:blank is valid"));
        let mut doc = html::parse(payload.html);
        let combined_css = collect_css_from_assets(&doc, &base_url, payload.assets);
        let stylesheet = css::parse(&combined_css);
        self.uses_interaction = stylesheet_uses_interaction(&stylesheet);
        self.images = load_images_from_assets(&doc, &base_url, payload.assets);
        let scripts = collect_scripts_from_assets(&doc, &base_url, payload.assets);

        self.js.reset();
        self.forms.reset();
        self.dirty = DirtyFlags::default();
        self.hovered_node = None;
        self.active_node = None;
        self.pressed_node = None;
        self.hover_href = None;
        self.title = doc
            .find_title()
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| {
                if payload.url.is_empty() {
                    "Droplet".to_string()
                } else {
                    payload.url.to_string()
                }
            });

        self.js.execute_scripts_with_dom(
            &scripts,
            &mut doc,
            &mut self.forms,
            &mut self.dirty,
            &mut self.title,
        );

        if let Some(updated) = doc.find_title().filter(|t| !t.is_empty()) {
            self.title = updated;
        }
        self.doc = Some(doc);
        self.stylesheet = Some(stylesheet);
        self.dirty = DirtyFlags::default();

        self.relayout();
        self.scroll_y = if payload.scroll_y.is_finite() {
            payload.scroll_y.clamp(0.0, self.max_scroll())
        } else {
            0.0
        };
        self.has_painted_frame = false;
        self.pending_damage = DamageRect::full(self.viewport_width, self.viewport_height);
    }

    /// Set the vertical scroll offset, clamped to the active document bounds.
    pub fn scroll(&mut self, scroll_y: f32) {
        if !scroll_y.is_finite() {
            return;
        }
        let clamped = scroll_y.clamp(0.0, self.max_scroll());
        if (clamped - self.scroll_y).abs() > f32::EPSILON {
            self.scroll_y = clamped;
            self.union_damage(viewport_damage_rect(
                self.viewport_width,
                self.viewport_height,
            ));
        }
    }

    /// Resize the renderer viewport and recompute layout if a document is loaded.
    pub fn resize(&mut self, width: usize, height: usize) {
        let w = width.max(1);
        let h = height.max(1);
        if w == self.viewport_width && h == self.viewport_height {
            return;
        }
        self.viewport_width = w;
        self.viewport_height = h;
        self.relayout();
        self.has_painted_frame = false;
        self.pending_damage = DamageRect::full(w, h);
    }

    /// Handle pointer movement at window-space coordinates `(x, y)`.
    pub fn mouse_move(&mut self, x: f32, y: f32) {
        let (hit_node, hit_href) = self.hit_test_viewport(x, y);
        if hit_href != self.hover_href {
            self.hover_href = hit_href;
            self.union_damage(self.status_damage_rect());
        }
        if hit_node != self.hovered_node {
            self.hovered_node = hit_node;
            if self.uses_interaction {
                self.relayout();
                self.union_damage(viewport_damage_rect(
                    self.viewport_width,
                    self.viewport_height,
                ));
            }
        }
    }

    /// Handle pointer button press at window-space coordinates `(x, y)`.
    pub fn mouse_down(&mut self, x: f32, y: f32) {
        let (hit_node, hit_href) = self.hit_test_viewport(x, y);
        self.pressed_node = hit_node;
        if hit_href != self.hover_href {
            self.hover_href = hit_href;
            self.union_damage(self.status_damage_rect());
        }
        if hit_node != self.active_node {
            self.active_node = hit_node;
            if self.uses_interaction {
                self.relayout();
                self.union_damage(viewport_damage_rect(
                    self.viewport_width,
                    self.viewport_height,
                ));
            }
        }
        if let Some(target) = self.focusable_ancestor_or_self(hit_node) {
            if self.forms.focused != Some(target) {
                self.forms.focused = Some(target);
                self.restyle_or_repaint_focus();
            }
        } else if self.forms.focused.is_some() {
            self.forms.focused = None;
            self.restyle_or_repaint_focus();
        }
    }

    /// Handle pointer button release at window-space coordinates `(x, y)`.
    pub fn mouse_up(&mut self, x: f32, y: f32) {
        let (hit_node, hit_href) = self.hit_test_viewport(x, y);
        let pressed = self.pressed_node.take();
        if hit_href != self.hover_href {
            self.hover_href = hit_href;
            self.union_damage(self.status_damage_rect());
        }
        if self.active_node.take().is_some() && self.uses_interaction {
            self.relayout();
            self.union_damage(viewport_damage_rect(
                self.viewport_width,
                self.viewport_height,
            ));
        }

        let Some(target) = hit_node else {
            return;
        };
        if pressed != Some(target) {
            return;
        }
        if let Some(mut doc) = self.doc.take() {
            let event = DomEventInput {
                event_type: "click".to_string(),
                target,
                key: String::new(),
                value: None,
                checked: None,
            };
            let outcome = self.js.dispatch_event_with_dom(
                &event,
                &mut doc,
                &mut self.forms,
                &mut self.dirty,
                &mut self.title,
            );
            if !outcome.default_prevented {
                if let Some(control) = self.control_target(target, &doc) {
                    self.activate_control(&mut doc, control);
                }
            }
            self.doc = Some(doc);
            self.flush_dirty();
        }
    }

    /// Evaluate JavaScript source against the loaded document DOM without I/O.
    pub fn eval_js(&mut self, source: &str) -> Result<String, String> {
        let Some(mut doc) = self.doc.take() else {
            return Err("no document loaded".to_string());
        };
        let runtime = self.js.runtime.get_or_insert_with(|| {
            let mut rt = JsRuntime::new();
            rt.install_dom_bindings();
            rt
        });
        let mut host = ShellDomHost {
            doc: &mut doc,
            forms: &mut self.forms,
            dirty: &mut self.dirty,
            logs: &mut self.js.logs,
            title: &mut self.title,
        };
        let eval_result = runtime.eval_with_host(source, &mut host);
        let _ = runtime.take_console_log();
        let out = match eval_result {
            Ok(value) => Ok(value.to_js_string()),
            Err(err) => {
                let msg = err.to_string();
                if self.js.errors.len() >= MAX_JS_LOG_ENTRIES {
                    let excess = self.js.errors.len() + 1 - MAX_JS_LOG_ENTRIES;
                    self.js.errors.drain(..excess);
                }
                self.js.errors.push(msg.clone());
                Err(msg)
            }
        };
        self.doc = Some(doc);
        self.flush_dirty();
        out
    }

    /// Paint the active document into caller-owned ARGB pixels and report frame metadata.
    pub fn paint_into_slice(
        &mut self,
        pixels: &mut [u32],
        width: usize,
        height: usize,
    ) -> RenderedFrameInfo {
        let w = width.max(1);
        let h = height.max(1);
        if w != self.viewport_width || h != self.viewport_height {
            self.resize(w, h);
        }
        self.flush_dirty();

        let needed = w.saturating_mul(h);
        if pixels.len() < needed {
            let damage = self.pending_damage.clamp(w, h);
            return RenderedFrameInfo {
                title: self.title.clone(),
                hover_href: self.hover_href.clone(),
                damage,
                layout_ms: self.last_layout_ms,
                paint_ms: 0.0,
                console_logs: self.js.logs.clone(),
            };
        }

        let paint_start = Instant::now();
        let damage = self.pending_damage.clamp(w, h);
        let title_slice = [self.title.as_str()];
        let display_list = self.display_list.as_deref().unwrap_or(&[]);
        let chrome = ChromeState {
            url_input: &self.url,
            url_focused: false,
            autocomplete_suffix: "",
            can_back: false,
            can_forward: false,
            status_text: self.hover_href.as_deref().unwrap_or(""),
            timing_badge: "",
            tab_titles: &title_slice,
            active_tab: 0,
            scroll_y: self.scroll_y,
            content_height: self.content_height,
        };

        let is_sub_rect = !damage.is_empty()
            && (damage.x > 0 || damage.y > 0 || damage.width < w || damage.height < h);
        let can_reuse_pixels =
            is_sub_rect && self.has_painted_frame && self.display_list.is_some() && pixels[0] != 0;

        if can_reuse_pixels {
            Canvas::with_external_slice_unfilled(w, h, pixels, |canvas| {
                render_frame_damaged(
                    canvas,
                    &mut self.font_engine,
                    display_list,
                    &chrome,
                    None,
                    damage,
                );
            });
        } else {
            Canvas::with_external_slice(w, h, pixels, Color::rgb(255, 255, 255), |canvas| {
                render_frame_ext(canvas, &mut self.font_engine, display_list, &chrome, None);
            });
        }

        self.has_painted_frame = true;
        self.pending_damage = DamageRect::default();

        RenderedFrameInfo {
            title: self.title.clone(),
            hover_href: self.hover_href.clone(),
            damage,
            layout_ms: self.last_layout_ms,
            paint_ms: paint_start.elapsed().as_secs_f32() * 1000.0,
            console_logs: self.js.logs.clone(),
        }
    }

    pub fn viewport_size(&self) -> (usize, usize) {
        (self.viewport_width, self.viewport_height)
    }

    pub fn scroll_y(&self) -> f32 {
        self.scroll_y
    }

    pub fn content_height(&self) -> f32 {
        self.content_height
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn hover_href(&self) -> Option<&str> {
        self.hover_href.as_deref()
    }

    pub fn console_logs(&self) -> &[String] {
        &self.js.logs
    }

    fn page_viewport_height(&self) -> f32 {
        self.viewport_height
            .saturating_sub(CHROME_HEIGHT + STATUS_BAR_HEIGHT) as f32
    }

    fn max_scroll(&self) -> f32 {
        (self.content_height - self.page_viewport_height()).max(0.0)
    }

    fn doc_point(&self, x: f32, y: f32) -> Option<(f32, f32)> {
        let top = CHROME_HEIGHT as f32;
        let bottom = self.viewport_height.saturating_sub(STATUS_BAR_HEIGHT) as f32;
        if x < 0.0 || x >= self.viewport_width as f32 || y < top || y >= bottom {
            return None;
        }
        Some((x, y - top + self.scroll_y))
    }

    fn hit_test_viewport(&self, x: f32, y: f32) -> (Option<NodeId>, Option<String>) {
        let Some((doc_x, doc_y)) = self.doc_point(x, y) else {
            return (None, None);
        };
        let Some(root) = self.layout_root.as_ref() else {
            return (None, None);
        };
        let node = droplet_layout::hit_test_node(root, doc_x, doc_y);
        let href = droplet_layout::hit_test_href(root, doc_x, doc_y).map(str::to_string);
        (node, href)
    }

    fn focusable_ancestor_or_self(&self, start: Option<NodeId>) -> Option<NodeId> {
        let doc = self.doc.as_ref()?;
        let mut curr = start;
        while let Some(id) = curr {
            if forms::is_focusable(doc, id) {
                return Some(id);
            }
            curr = doc.get(id).and_then(|n| n.parent);
        }
        None
    }

    fn control_target(&self, hit: NodeId, doc: &Document) -> Option<NodeId> {
        let mut curr = Some(hit);
        while let Some(id) = curr {
            if forms::control_kind(doc, id).is_some() {
                return Some(id);
            }
            curr = doc.get(id).and_then(|n| n.parent);
        }
        None
    }

    fn activate_control(&mut self, doc: &mut Document, node: NodeId) {
        let changed = match forms::control_kind(doc, node) {
            Some(ControlKind::Checkbox) => forms::toggle_checked(doc, node),
            Some(ControlKind::Radio) => forms::check_radio(doc, node),
            Some(ControlKind::Select) => forms::cycle_select(doc, node, 1),
            _ => false,
        };
        if changed {
            self.dirty.style_and_layout = true;
        }
    }

    fn restyle_or_repaint_focus(&mut self) {
        if self.uses_interaction {
            self.relayout();
        } else {
            self.refresh_display_list();
        }
        self.union_damage(viewport_damage_rect(
            self.viewport_width,
            self.viewport_height,
        ));
    }

    fn status_damage_rect(&self) -> DamageRect {
        let h = self.viewport_height;
        let vp_bottom = h.saturating_sub(STATUS_BAR_HEIGHT);
        let top = vp_bottom.saturating_sub(34).max(CHROME_HEIGHT.min(vp_bottom));
        DamageRect::new(
            0,
            top,
            self.viewport_width.min(584),
            vp_bottom.saturating_sub(top),
        )
    }

    fn union_damage(&mut self, rect: DamageRect) {
        let clamped = rect.clamp(self.viewport_width, self.viewport_height);
        if !clamped.is_empty() {
            self.pending_damage = self.pending_damage.union(clamped);
        }
    }

    fn style_context(&self) -> StyleContext {
        let focused_node = self
            .forms
            .focused
            .filter(|id| self.doc.as_ref().and_then(|d| d.get(*id)).is_some());
        StyleContext {
            viewport_width: self.viewport_width as f32,
            viewport_height: self.page_viewport_height().max(1.0),
            hovered_node: self.hovered_node,
            active_node: self.active_node,
            focused_node,
        }
    }

    fn relayout(&mut self) {
        let (Some(doc), Some(stylesheet)) = (self.doc.as_ref(), self.stylesheet.as_ref()) else {
            return;
        };
        let start = Instant::now();
        let ctx = self.style_context();
        let visual = self.forms.visual(doc);
        let layout_root = compute_layout_with_ctx(doc, stylesheet, &ctx, &self.images);
        let display_list = build_display_list_with_forms(&layout_root, &visual);
        let content_height = compute_content_height(&layout_root);
        let page_vh = self.page_viewport_height();
        let max_scroll = (content_height - page_vh).max(0.0);
        self.scroll_y = self.scroll_y.clamp(0.0, max_scroll);
        self.content_height = content_height;
        self.layout_root = Some(layout_root);
        self.display_list = Some(display_list);
        self.dirty.style_and_layout = false;
        self.dirty.paint = false;
        self.last_layout_ms = start.elapsed().as_secs_f32() * 1000.0;
    }

    fn refresh_display_list(&mut self) {
        let visual = match (self.doc.as_ref(), self.layout_root.as_ref()) {
            (Some(doc), Some(_)) => self.forms.visual(doc),
            _ => FormVisualState::default(),
        };
        if let Some(root) = self.layout_root.as_ref() {
            self.display_list = Some(build_display_list_with_forms(root, &visual));
        }
        self.dirty.paint = false;
    }

    fn flush_dirty(&mut self) {
        let dirty = self.dirty.take();
        if dirty.is_clean() {
            return;
        }
        if dirty.title_changed {
            if let Some(next_title) = self.doc.as_ref().and_then(Document::find_title) {
                self.title = next_title;
            }
            self.union_damage(chrome_damage_rect(self.viewport_width, self.viewport_height));
        }
        if dirty.style_and_layout {
            self.relayout();
            self.union_damage(viewport_damage_rect(
                self.viewport_width,
                self.viewport_height,
            ));
        } else if dirty.paint {
            self.refresh_display_list();
            self.union_damage(viewport_damage_rect(
                self.viewport_width,
                self.viewport_height,
            ));
        }
    }
}

fn doc_preorder_nodes(doc: &Document) -> Vec<NodeId> {
    let mut order = Vec::with_capacity(doc.nodes.len());
    let mut stack = vec![doc.root()];
    while let Some(id) = stack.pop() {
        order.push(id);
        if let Some(node) = doc.get(id) {
            for &child in node.children.iter().rev() {
                stack.push(child);
            }
        }
    }
    order
}

fn collect_css_from_assets(
    doc: &Document,
    base_url: &Url,
    assets: &[(String, Vec<u8>)],
) -> String {
    let mut combined = String::new();
    for id in doc_preorder_nodes(doc) {
        let Some(node) = doc.get(id) else {
            continue;
        };
        let NodeKind::Element(data) = &node.kind else {
            continue;
        };
        if data.tag_name.eq_ignore_ascii_case("style") {
            for &child_id in &node.children {
                if let Some(child) = doc.get(child_id) {
                    if let NodeKind::Text(text) = &child.kind {
                        combined.push_str(text);
                        combined.push('\n');
                    }
                }
            }
        } else if data.tag_name.eq_ignore_ascii_case("link") {
            let is_stylesheet = data.attr("rel").is_some_and(|rel| {
                rel.split_ascii_whitespace()
                    .any(|token| token.eq_ignore_ascii_case("stylesheet"))
            });
            if !is_stylesheet {
                continue;
            }
            let Some(href) = data.attr("href") else {
                continue;
            };
            if let Some(bytes) = resolve_asset_bytes(assets, base_url, href) {
                combined.push_str(&String::from_utf8_lossy(&bytes));
                combined.push('\n');
            }
        }
    }
    combined
}

fn load_images_from_assets(
    doc: &Document,
    base_url: &Url,
    assets: &[(String, Vec<u8>)],
) -> HashMap<String, Arc<DecodedImage>> {
    let mut images = HashMap::new();
    for id in doc_preorder_nodes(doc) {
        let Some(node) = doc.get(id) else {
            continue;
        };
        let NodeKind::Element(data) = &node.kind else {
            continue;
        };
        if !data.tag_name.eq_ignore_ascii_case("img") {
            continue;
        }
        let Some(raw_src) = data.attr("src") else {
            continue;
        };
        let key = raw_src.trim();
        if key.is_empty() || images.contains_key(raw_src) {
            continue;
        }
        let Some(bytes) = resolve_asset_bytes(assets, base_url, key) else {
            continue;
        };
        if let Ok(decoded) = droplet_image::decode(&bytes) {
            images.insert(raw_src.to_string(), Arc::new(decoded));
        }
    }
    images
}

fn collect_scripts_from_assets(
    doc: &Document,
    base_url: &Url,
    assets: &[(String, Vec<u8>)],
) -> Vec<ScriptUnit> {
    let mut out = Vec::new();
    for id in doc_preorder_nodes(doc) {
        if out.len() >= MAX_SCRIPTS_PER_PAGE {
            break;
        }
        let Some(node) = doc.get(id) else {
            continue;
        };
        let NodeKind::Element(data) = &node.kind else {
            continue;
        };
        if !data.tag_name.eq_ignore_ascii_case("script") {
            continue;
        }
        if !is_javascript_script_type(data.attr("type")) {
            continue;
        }
        if let Some(raw_src) = data.attr("src") {
            let trimmed = raw_src.trim();
            if trimmed.is_empty() {
                continue;
            }
            let resolved_url = base_url
                .resolve(trimmed)
                .map(|u| u.as_str().to_string())
                .unwrap_or_else(|_| trimmed.to_string());
            match resolve_asset_bytes(assets, base_url, trimmed) {
                Some(bytes) => {
                    let code = String::from_utf8_lossy(&bytes).into_owned();
                    out.push(ScriptUnit::External {
                        url: resolved_url,
                        code,
                    });
                }
                None => out.push(ScriptUnit::LoadError {
                    origin: resolved_url,
                    message: "missing pre-fetched script asset".to_string(),
                }),
            }
        } else {
            let mut code = String::new();
            for &child_id in &node.children {
                if let Some(child) = doc.get(child_id) {
                    if let NodeKind::Text(text) = &child.kind {
                        code.push_str(text);
                    }
                }
            }
            if !code.is_empty() {
                out.push(ScriptUnit::Inline { code });
            }
        }
    }
    out
}

fn resolve_asset_bytes<'a>(
    assets: &'a [(String, Vec<u8>)],
    base_url: &Url,
    raw_ref: &str,
) -> Option<Cow<'a, [u8]>> {
    let trimmed = raw_ref.trim();
    if trimmed.is_empty() {
        return None;
    }
    if is_data_uri(trimmed) {
        return decode_data_uri_bytes(trimmed).map(Cow::Owned);
    }
    if let Ok(resolved_url) = base_url.resolve(trimmed) {
        let resolved_str = resolved_url.as_str();
        if let Some((_, bytes)) = assets.iter().find(|(k, _)| k == resolved_str) {
            return Some(Cow::Borrowed(bytes.as_slice()));
        }
    }
    assets
        .iter()
        .find(|(k, _)| k == trimmed)
        .map(|(_, bytes)| Cow::Borrowed(bytes.as_slice()))
}

fn is_javascript_script_type(type_attr: Option<&str>) -> bool {
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

fn is_data_uri(uri: &str) -> bool {
    uri.trim()
        .as_bytes()
        .get(..5)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"data:"))
}

fn decode_data_uri_bytes(uri: &str) -> Option<Vec<u8>> {
    let trimmed = uri.trim();
    if !is_data_uri(trimmed) {
        return None;
    }
    let rest = &trimmed[5..];
    let (meta, body) = rest.split_once(',')?;
    let is_base64 = meta
        .split(';')
        .any(|part| part.trim().eq_ignore_ascii_case("base64"));
    if is_base64 {
        decode_base64_bytes(body)
    } else {
        Some(percent_decode_bytes(body))
    }
}

fn decode_base64_bytes(input: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0u8;
    let mut saw_padding = false;
    for b in input.bytes() {
        if b.is_ascii_whitespace() {
            continue;
        }
        if b == b'=' {
            saw_padding = true;
            continue;
        }
        if saw_padding {
            return None;
        }
        let val = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        };
        buf = (buf << 6) | u32::from(val);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xFF) as u8);
        }
    }
    Some(out)
}

fn percent_decode_bytes(input: &str) -> Vec<u8> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ppm_red_2x2() -> Vec<u8> {
        b"P6\n2 2\n255\n\xff\x00\x00\xff\x00\x00\xff\x00\x00\xff\x00\x00".to_vec()
    }

    #[test]
    fn headless_renderer_renders_html_asset_css_image_and_scripts_zero_io() {
        let html = r#"<!DOCTYPE html>
<html>
  <head>
    <title>Start Title</title>
    <link rel="stylesheet" href="/styles/theme.css">
  </head>
  <body>
    <div id="banner">Initial</div>
    <img id="logo" src="/img/red.ppm" width="40" height="40">
    <a id="cta" href="/next" style="display:block;width:120px;height:30px;">Next</a>
    <script src="/app/boot.js"></script>
    <script>
      document.title = "Updated Title";
      console.log("inline-ran");
    </script>
  </body>
</html>"#;

        let assets = vec![
            (
                "https://example.com/styles/theme.css".to_string(),
                b"body { background-color: #102030; margin: 0; } #banner { color: #ffffff; }"
                    .to_vec(),
            ),
            ("https://example.com/img/red.ppm".to_string(), ppm_red_2x2()),
            (
                "https://example.com/app/boot.js".to_string(),
                b"document.getElementById('banner').textContent = 'FromAssetJS'; console.log('asset-ran');"
                    .to_vec(),
            ),
        ];

        let mut renderer = HeadlessRenderer::new(480, 320);
        renderer.load_payload(RenderPayload {
            url: "https://example.com/home",
            html,
            assets: &assets,
            viewport_width: 480,
            viewport_height: 320,
            scroll_y: 0.0,
        });

        let mut pixels = vec![0u32; 480 * 320];
        let info = renderer.paint_into_slice(&mut pixels, 480, 320);

        assert_eq!(info.title, "Updated Title");
        assert_eq!(info.damage, DamageRect::full(480, 320));
        assert!(info.console_logs.iter().any(|l| l == "asset-ran"));
        assert!(info.console_logs.iter().any(|l| l == "inline-ran"));

        // Decoded asset image is loaded in-memory without touching disk/net.
        assert!(renderer.images.contains_key("/img/red.ppm"));
        // Red PPM pixels show up in the rendered viewport area (below CHROME_HEIGHT).
        let red_xrgb = Color::rgb(0xFF, 0x00, 0x00).to_u32_xrgb();
        assert!(
            pixels[CHROME_HEIGHT * 480..].contains(&red_xrgb),
            "expected decoded asset image pixels in viewport"
        );
        // Dark theme CSS background (#102030 -> 0x00102030) painted in document body.
        let bg_xrgb = Color::rgb(0x10, 0x20, 0x30).to_u32_xrgb();
        assert!(pixels[CHROME_HEIGHT * 480..].contains(&bg_xrgb));
    }

    #[test]
    fn headless_renderer_incremental_damage_scroll_resize_and_eval_js() {
        let html = r#"<!DOCTYPE html>
<html>
  <head><title>Dynamic</title></head>
  <body style="margin:0;">
    <a id="link" href="https://example.com/dest" style="display:block;width:200px;height:40px;background:#ffffff;">Go Dest Link</a>
    <div id="tall" style="height:900px;background:#334455;">Tall</div>
  </body>
</html>"#;

        let mut renderer = HeadlessRenderer::new(400, 300);
        renderer.load_payload(RenderPayload {
            url: "https://example.com/",
            html,
            assets: &[],
            viewport_width: 400,
            viewport_height: 300,
            scroll_y: 0.0,
        });

        let mut pixels = vec![0u32; 400 * 300];
        let frame1 = renderer.paint_into_slice(&mut pixels, 400, 300);
        assert_eq!(frame1.damage, DamageRect::full(400, 300));

        // Hover over link -> hover_href updates & tight status damage.
        let link_id = renderer
            .doc
            .as_ref()
            .unwrap()
            .get_element_by_id("link")
            .unwrap();
        let link_box = droplet_layout::find_layout_box_by_node(
            renderer.layout_root.as_ref().unwrap(),
            link_id,
        )
        .expect("link layout box")
        .dimensions
        .border_box();
        renderer.mouse_move(
            link_box.x + link_box.width * 0.5,
            CHROME_HEIGHT as f32 + link_box.y + link_box.height * 0.5,
        );
        let frame2 = renderer.paint_into_slice(&mut pixels, 400, 300);
        assert_eq!(frame2.hover_href.as_deref(), Some("https://example.com/dest"));
        assert!(frame2.damage.y >= 300 - 34 && !frame2.damage.is_empty());

        // Scroll -> viewport damage only (not full canvas) & reused slice updates cleanly.
        renderer.scroll(120.0);
        assert!((renderer.scroll_y() - 120.0).abs() < 0.01);
        let frame3 = renderer.paint_into_slice(&mut pixels, 400, 300);
        assert_eq!(frame3.damage, viewport_damage_rect(400, 300));

        // JS eval updates title + DOM + console logs.
        renderer
            .eval_js("document.title = 'JS Title'; document.getElementById('tall').textContent = 'Привет'; console.log('eval-ok');")
            .unwrap();
        let tall_id = renderer
            .doc
            .as_ref()
            .unwrap()
            .get_element_by_id("tall")
            .unwrap();
        assert_eq!(
            renderer.doc.as_ref().unwrap().text_content_of(tall_id),
            "Привет"
        );
        let frame4 = renderer.paint_into_slice(&mut pixels, 400, 300);
        assert_eq!(frame4.title, "JS Title");
        assert!(frame4.console_logs.iter().any(|l| l == "eval-ok"));

        // Resize emits full damage for new dimensions.
        let mut big_pixels = vec![0u32; 500 * 360];
        let frame5 = renderer.paint_into_slice(&mut big_pixels, 500, 360);
        assert_eq!(renderer.viewport_size(), (500, 360));
        assert_eq!(frame5.damage, DamageRect::full(500, 360));
    }

    #[test]
    fn headless_renderer_decodes_data_uri_assets_without_network() {
        let css_uri = "data:text/css;base64,Ym9keSB7IGJhY2tncm91bmQtY29sb3I6ICMwMGEwMDA7IH0=";
        let html = format!(
            "<!DOCTYPE html><html><head><link rel=\"stylesheet\" href=\"{css_uri}\"></head><body>Hi</body></html>"
        );
        let mut renderer = HeadlessRenderer::new(320, 240);
        renderer.load_payload(RenderPayload {
            url: "https://example.com/",
            html: &html,
            assets: &[],
            viewport_width: 320,
            viewport_height: 240,
            scroll_y: 0.0,
        });
        let mut pixels = vec![0u32; 320 * 240];
        let _ = renderer.paint_into_slice(&mut pixels, 320, 240);
        assert!(pixels[CHROME_HEIGHT * 320..].contains(&Color::rgb(0x00, 0xA0, 0x00).to_u32_xrgb()));
    }
}
