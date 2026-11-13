pub mod bench;
mod bookmarks;
pub mod find;
pub mod forms;
pub mod history_store;
pub mod js;
pub mod loader;
pub mod nav;
pub mod renderer;
pub mod tab;
pub mod webview;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use droplet_dom::{html, Document, NodeId, NodeKind};
use droplet_font::FontEngine;
use droplet_gfx::{Canvas, Color, DamageRect};
use droplet_image::DecodedImage;
use droplet_js::{DomEventInput, DomEventOutcome};
use droplet_layout::LayoutBox;
use droplet_paint::{
    build_display_list_with_forms, chrome_damage_rect, chrome_hitboxes, render_frame_damaged,
    render_frame_ext, viewport_damage_rect, BoxModelOverlay, ChromeState, FindBarView,
    FindHighlight, FormVisualState, OverlayState, CHROME_HEIGHT, STATUS_BAR_HEIGHT,
};
use droplet_platform::{CaretMove, PlatformApp};
use droplet_style::{css, StyleContext, Stylesheet};
use droplet_url::Url;

use find::FindState;
use history_store::HistoryStore;

pub use droplet_platform::NavKey;
pub use js::{ScriptUnit, TabJsState};
pub use renderer::{HeadlessRenderer, RenderPayload, RenderedFrameInfo};
pub use tab::{DirtyFlags, TabState, MAX_WARM_TABS};

const HOME_URL: &str = "about:droplet";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChromeTarget {
    TabClose(usize),
    Tab(usize),
    NewTabBtn,
    BackBtn,
    ForwardBtn,
    ReloadBtn,
    HomeBtn,
    AddressBar,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PressedTarget {
    Chrome(ChromeTarget),
    Viewport {
        href: Option<String>,
        control: Option<NodeId>,
        node: Option<NodeId>,
    },
}

static JS_DEFAULT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Sets the process-wide default for [`BrowserOptions::js`] (the `--js` CLI flag).
///
/// JavaScript is off by default: the ES-subset VM cannot run real-world scripts yet
/// (see ROADMAP Phase 7), and skipping scripts saves network round-trips and memory.
pub fn set_js_default(enabled: bool) {
    JS_DEFAULT.store(enabled, std::sync::atomic::Ordering::Relaxed);
}

/// Per-browser feature switches fixed at construction time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BrowserOptions {
    /// Collect, fetch and execute `<script>` elements.
    pub js: bool,
}

impl BrowserOptions {
    /// Options derived from process-wide defaults (see [`set_js_default`]).
    pub fn from_defaults() -> Self {
        Self {
            js: JS_DEFAULT.load(std::sync::atomic::Ordering::Relaxed),
        }
    }
}

/// Top-level browser shell managing multi-tab sessions, low-RAM tab hibernation,
/// navigation history, pointer/keyboard interaction, and frame rendering.
pub struct Browser {
    pub tabs: Vec<TabState>,
    pub active_tab: usize,
    pub viewport_width: usize,
    pub viewport_height: usize,
    pub url_input: String,
    pub url_focused: bool,
    pub status_text: String,
    pub find: FindState,
    pub inspector_enabled: bool,
    pub(crate) store: HistoryStore,
    base_status_text: String,
    pub needs_repaint: bool,
    pending_damage: Option<DamageRect>,
    staged_render_damage: Option<DamageRect>,
    present_backend: &'static str,
    font_engine: FontEngine,
    hovered_node: Option<NodeId>,
    active_node: Option<NodeId>,
    pressed_target: Option<PressedTarget>,
    active_seq_counter: u64,
    js_enabled: bool,
    nav: nav::Navigator,
    /// Initial URL of a GUI browser, loaded once the event loop exists (see `new_deferred`).
    deferred_start: Option<String>,
    /// Native browser window that WebKit helpers embed into (DROP-170); `None` headless.
    embed_parent: Option<u64>,
}

impl Browser {
    pub fn new(initial_url: &str, width: usize, height: usize) -> Self {
        Self::with_options(initial_url, width, height, BrowserOptions::from_defaults())
    }

    /// GUI constructor: the window opens immediately on a blank tab and `initial_url` starts
    /// loading when the platform installs its waker (in the background). Backends without a
    /// waker load it synchronously right before the first frame.
    pub fn new_deferred(initial_url: &str, width: usize, height: usize) -> Self {
        let mut browser = Self::new("about:blank", width, height);
        browser.url_input = initial_url.to_string();
        browser.deferred_start = Some(initial_url.to_string());
        browser
    }

    fn start_deferred(&mut self) {
        if let Some(url) = self.deferred_start.take() {
            self.load_input_into_tab(0, &url, false);
        }
    }

    /// Constructs a browser with explicit feature switches.
    pub fn with_options(
        initial_url: &str,
        width: usize,
        height: usize,
        options: BrowserOptions,
    ) -> Self {
        Self::with_history_store(
            initial_url,
            width,
            height,
            HistoryStore::from_env(),
            options,
        )
    }

    /// Constructs a browser with an isolated persistent store directory.
    pub fn with_data_dir(
        initial_url: &str,
        width: usize,
        height: usize,
        data_dir: impl Into<PathBuf>,
    ) -> Self {
        Self::with_history_store(
            initial_url,
            width,
            height,
            HistoryStore::open(data_dir),
            BrowserOptions::from_defaults(),
        )
    }

    fn with_history_store(
        initial_url: &str,
        width: usize,
        height: usize,
        store: HistoryStore,
        options: BrowserOptions,
    ) -> Self {
        // Persistent cookies live next to history (DROP-113); loads them into the
        // process-wide jar and makes later `cookies::flush` calls save there.
        droplet_net::cookies::attach_store(store.data_dir());
        let initial_tab = TabState::create_tab("Droplet", 1);

        let mut browser = Self {
            tabs: vec![initial_tab],
            active_tab: 0,
            viewport_width: width.max(1),
            viewport_height: height.max(1),
            url_input: initial_url.to_string(),
            url_focused: false,
            status_text: String::new(),
            find: FindState::default(),
            inspector_enabled: false,
            store,
            base_status_text: String::new(),
            needs_repaint: true,
            pending_damage: Some(DamageRect::full(width.max(1), height.max(1))),
            staged_render_damage: None,
            present_backend: "software",
            font_engine: FontEngine::new(),
            hovered_node: None,
            active_node: None,
            pressed_target: None,
            active_seq_counter: 1,
            js_enabled: options.js,
            nav: nav::Navigator::new(),
            deferred_start: None,
            embed_parent: None,
        };

        browser.load_input_into_tab(0, initial_url, false);
        browser
    }

    pub fn active_title(&self) -> &str {
        self.tabs
            .get(self.active_tab)
            .map(|t| t.title.as_str())
            .unwrap_or("Droplet")
    }

    pub fn js_enabled(&self) -> bool {
        self.js_enabled
    }

    pub fn present_backend(&self) -> &'static str {
        self.present_backend
    }

    pub fn pending_damage(&self) -> Option<DamageRect> {
        if !self.needs_repaint {
            return None;
        }
        self.pending_damage
            .or_else(|| Some(DamageRect::full(self.viewport_width, self.viewport_height)))
            .map(|d| d.clamp(self.viewport_width, self.viewport_height))
            .filter(|d| !d.is_empty())
    }

    fn record_damage(&mut self, rect: DamageRect) {
        let was_dirty = self.needs_repaint;
        self.needs_repaint = true;
        let clamped = rect.clamp(self.viewport_width, self.viewport_height);
        if clamped.is_empty() {
            return;
        }
        self.pending_damage = if was_dirty {
            Some(match self.pending_damage {
                Some(existing) if !existing.is_empty() => existing.union(clamped),
                _ => clamped,
            })
        } else {
            Some(clamped)
        };
    }

    fn mark_full_repaint(&mut self) {
        self.record_damage(DamageRect::full(self.viewport_width, self.viewport_height));
    }

    fn mark_chrome_repaint(&mut self) {
        self.record_damage(chrome_damage_rect(self.viewport_width, self.viewport_height));
    }

    fn mark_viewport_repaint(&mut self) {
        self.record_damage(viewport_damage_rect(
            self.viewport_width,
            self.viewport_height,
        ));
    }

    fn mark_status_repaint(&mut self) {
        let h = self.viewport_height;
        let vp_bottom = h.saturating_sub(STATUS_BAR_HEIGHT);
        let top = vp_bottom.saturating_sub(34).max(CHROME_HEIGHT.min(vp_bottom));
        self.record_damage(DamageRect::new(
            0,
            top,
            self.viewport_width.min(584),
            vp_bottom.saturating_sub(top),
        ));
    }

    pub fn active_url_str(&self) -> &str {
        self.tabs
            .get(self.active_tab)
            .map(|t| t.current_url.as_str())
            .unwrap_or("about:blank")
    }

    pub fn active_dom_dump(&self) -> String {
        self.tabs
            .get(self.active_tab)
            .and_then(|t| t.doc.as_ref())
            .map(Document::dump_tree)
            .unwrap_or_default()
    }

    pub fn active_layout_dump(&self) -> String {
        self.tabs
            .get(self.active_tab)
            .and_then(|t| t.layout_root.as_ref())
            .map(droplet_layout::dump_layout)
            .unwrap_or_default()
    }

    pub fn active_js_logs(&self) -> &[String] {
        self.tabs
            .get(self.active_tab)
            .map(|t| t.js.logs.as_slice())
            .unwrap_or(&[])
    }

    pub fn active_js_errors(&self) -> &[String] {
        self.tabs
            .get(self.active_tab)
            .map(|t| t.js.errors.as_slice())
            .unwrap_or(&[])
    }

    /// Advances the active tab's deterministic JS timer clock to `now_ms`, firing timers
    /// with live DOM access and flushing only dirty layout/paint layers.
    pub fn tick_js(&mut self, now_ms: f64) {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return;
        };
        if let Some(doc) = tab.doc.as_mut() {
            tab.js.tick_with_dom(
                now_ms,
                doc,
                &mut tab.forms,
                &mut tab.dirty,
                &mut tab.title,
            );
        } else {
            tab.js.tick(now_ms);
        }
        self.flush_active_dirty();
        self.sync_active_js_status();
    }

    /// Drives one browser event-loop tick at `now_ms`.
    pub fn tick(&mut self, now_ms: f64) {
        self.tick_js(now_ms);
    }

    /// Advances the active tab's deterministic JS timer clock by `delta_ms`.
    pub fn advance_js(&mut self, delta_ms: f64) {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return;
        };
        if let Some(doc) = tab.doc.as_mut() {
            tab.js.advance_with_dom(
                delta_ms,
                doc,
                &mut tab.forms,
                &mut tab.dirty,
                &mut tab.title,
            );
        } else {
            tab.js.advance(delta_ms);
        }
        self.flush_active_dirty();
        self.sync_active_js_status();
    }

    /// Evaluates JavaScript in the active tab with live DOM access and flushes dirty state.
    pub fn eval_active_js(&mut self, code: &str) -> Result<(), String> {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return Err("no active tab".to_string());
        };
        let res = if let Some(doc) = tab.doc.as_mut() {
            tab.js.eval_with_dom(
                code,
                doc,
                &mut tab.forms,
                &mut tab.dirty,
                &mut tab.title,
            )
        } else {
            Err("active tab has no document".to_string())
        };
        self.flush_active_dirty();
        self.sync_active_js_status();
        res
    }

    /// Dispatches a DOM event with bubbling on the active tab and flushes dirty state.
    pub fn dispatch_dom_event_on(
        &mut self,
        target: NodeId,
        event_type: &str,
        key: &str,
    ) -> DomEventOutcome {
        self.dispatch_dom_event(event_type, target, key)
    }

    /// Dispatches a `keydown` DOM event for `key` if a page control is focused.
    pub fn handle_key_down(&mut self, key: &str) -> DomEventOutcome {
        if let Some(node) = self.focused_control() {
            self.dispatch_dom_event("keydown", node, key)
        } else {
            DomEventOutcome::default()
        }
    }

    /// Dispatches a `keyup` DOM event for `key` if a page control is focused.
    pub fn handle_key_up(&mut self, key: &str) -> DomEventOutcome {
        if let Some(node) = self.focused_control() {
            self.dispatch_dom_event("keyup", node, key)
        } else {
            DomEventOutcome::default()
        }
    }

    fn dispatch_dom_event(
        &mut self,
        event_type: &str,
        target: NodeId,
        key: &str,
    ) -> DomEventOutcome {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return DomEventOutcome::default();
        };
        if tab.js.runtime.is_none() {
            return DomEventOutcome::default();
        }
        let Some(doc) = tab.doc.as_mut() else {
            return DomEventOutcome::default();
        };
        let value = forms::control_kind(doc, target).map(|_| forms::value_of(doc, target));
        let checked = forms::control_kind(doc, target).and_then(|kind| {
            matches!(
                kind,
                droplet_layout::ControlKind::Checkbox | droplet_layout::ControlKind::Radio
            )
            .then(|| doc.element(target).is_some_and(|e| e.attr("checked").is_some()))
        });
        let input = DomEventInput {
            event_type: event_type.to_string(),
            target,
            key: key.to_string(),
            value,
            checked,
        };
        let outcome = tab.js.dispatch_event_with_dom(
            &input,
            doc,
            &mut tab.forms,
            &mut tab.dirty,
            &mut tab.title,
        );
        self.flush_active_dirty();
        self.sync_active_js_status();
        outcome
    }

    /// Extension hook for DROP-008 to attach DOM/host bindings onto the active tab's JS runtime.
    pub fn configure_active_js<F>(&mut self, hook: F)
    where
        F: FnMut(&mut droplet_js::JsRuntime),
    {
        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
            tab.js.ensure_runtime_with_hook(hook);
        }
    }

    fn sync_active_js_status(&mut self) {
        let Some(tab) = self.tabs.get(self.active_tab) else {
            return;
        };
        let next_base = tab.js.status_summary().unwrap_or_default();
        if next_base != self.base_status_text {
            let was_showing_base = self.status_text == self.base_status_text;
            self.base_status_text = next_base;
            if was_showing_base {
                self.status_text = self.base_status_text.clone();
            }
            self.mark_status_repaint();
        }
    }

    pub fn navigate(&mut self, input: &str) {
        self.load_input_into_tab(self.active_tab, input, true);
    }

    fn load_input_into_tab(&mut self, tab_idx: usize, input: &str, push_history: bool) {
        self.store.dismiss_autocomplete();
        let trimmed = input.trim();
        let current_url = &self.tabs[tab_idx].current_url;
        let resolved =
            if !push_history || (current_url.is_about() && !trimmed.starts_with('#')) {
                Url::parse(trimmed)
            } else {
                current_url.resolve(trimmed)
            };

        match resolved {
            Ok(target_url) => {
                let history = if push_history {
                    nav::HistoryOp::Push
                } else {
                    nav::HistoryOp::None
                };
                self.start_navigation(tab_idx, target_url, history, nav::NavRequest::Get, None);
            }
            Err(err) => {
                // A newer navigation supersedes anything still in flight for this tab.
                self.tabs[tab_idx].pending = None;
                if tab_idx == self.active_tab {
                    self.url_input = trimmed.to_string();
                    self.url_focused = false;
                }
                let base_url = self.tabs[tab_idx].current_url.clone();
                let load_start = Instant::now();
                let page = loader::build_navigation_error_page(
                    base_url,
                    trimmed,
                    &err.to_string(),
                );
                self.apply_loaded_page(tab_idx, page, load_start);
            }
        }
    }

    /// Starts loading `url` into `tab_idx`. The page currently shown stays interactive until
    /// the result is committed; history changes only on commit (see [`nav::HistoryOp`]).
    fn start_navigation(
        &mut self,
        tab_idx: usize,
        url: Url,
        history: nav::HistoryOp,
        request: nav::NavRequest,
        restore_scroll: Option<f32>,
    ) {
        self.store.dismiss_autocomplete();
        if tab_idx == self.active_tab {
            self.url_input = url.as_str().to_string();
            self.url_focused = false;
        }
        if request == nav::NavRequest::Get && self.route_to_webview(tab_idx, &url, history) {
            return;
        }
        // Leaving WebKit territory from the address bar: the helper is no longer needed.
        self.tabs[tab_idx].webview = None;
        let pending = nav::PendingNav {
            nav_id: self.nav.next_id(),
            url: url.clone(),
            history,
            restore_scroll,
            start: Instant::now(),
        };

        // Built-in history/bookmark pages need the store: always local and instant.
        if request == nav::NavRequest::Get {
            if let Some(page) = self.store.user_page(&url) {
                self.tabs[tab_idx].pending = None;
                self.commit_navigation(tab_idx, pending, page);
                return;
            }
        }

        let nav_id = pending.nav_id;
        self.tabs[tab_idx].pending = Some(pending);
        match self.nav.spawn(nav_id, url, request, self.js_enabled) {
            Ok(()) => {
                if tab_idx == self.active_tab {
                    self.status_text = loading_status(&self.tabs[tab_idx]);
                    self.mark_chrome_repaint();
                }
            }
            Err(inline) => {
                let (url, request) = *inline;
                let page = nav::fetch_page(&url, &request, self.js_enabled);
                if let Some(pending) = self.tabs[tab_idx].pending.take() {
                    self.commit_navigation(tab_idx, pending, page);
                }
            }
        }
    }

    /// Opens `url` in the tab's WebKit view when the site needs a full web platform and the
    /// platform can embed one. Returns `false` to let Droplet's own engine load it.
    fn route_to_webview(&mut self, tab_idx: usize, url: &Url, history: nav::HistoryOp) -> bool {
        let Some(parent) = self.embed_parent else {
            return false;
        };
        if !webview::wants_webview(url) {
            return false;
        }
        let waker = self.nav.waker();
        let tab = &mut self.tabs[tab_idx];
        tab.pending = None;
        match tab.webview.as_mut() {
            Some(view) => view.load(url.as_str()),
            None => match webview::WebView::spawn(parent, url.as_str(), waker) {
                Ok(view) => {
                    // The previous Droplet-rendered page is history now: free it.
                    tab.clear_warm_payload();
                    tab.webview = Some(view);
                }
                Err(err) => {
                    eprintln!("droplet: WebKit view unavailable ({err}); using built-in engine");
                    return false;
                }
            },
        }
        let previous = tab.current_url.clone();
        match history {
            nav::HistoryOp::Push if previous.as_str() != url.as_str() => {
                tab.back_stack.push(previous);
                tab.forward_stack.clear();
            }
            nav::HistoryOp::Back => {
                tab.back_stack.pop();
                tab.forward_stack.push(previous);
            }
            nav::HistoryOp::Forward => {
                tab.forward_stack.pop();
                tab.back_stack.push(previous);
            }
            _ => {}
        }
        tab.hibernated = false;
        tab.current_url = url.clone();
        tab.title = url.host.clone();
        tab.scroll_y = 0.0;
        tab.content_height = 0.0;
        if tab_idx == self.active_tab {
            self.status_text = "WebKit: загрузка…".to_string();
            self.base_status_text.clear();
            self.mark_full_repaint();
        }
        self.enforce_warm_tab_limit();
        true
    }

    /// Applies events from WebKit helpers (title, URL, load state, crash).
    /// Returns `true` if anything visible changed.
    pub fn poll_webviews(&mut self) -> bool {
        let mut changed = false;
        for idx in 0..self.tabs.len() {
            let Some(view) = self.tabs[idx].webview.as_mut() else {
                continue;
            };
            let events = view.drain();
            for event in events {
                changed = true;
                let active = idx == self.active_tab;
                match event {
                    webview::WebViewEvent::Uri(raw) => {
                        if let Ok(url) = Url::parse(&raw) {
                            self.tabs[idx].current_url = url;
                            if active && !self.url_focused {
                                self.url_input = raw;
                            }
                        }
                    }
                    webview::WebViewEvent::Title(title) if !title.is_empty() => {
                        self.tabs[idx].title = title;
                    }
                    webview::WebViewEvent::Started if active => {
                        self.status_text = "WebKit: загрузка…".to_string();
                    }
                    webview::WebViewEvent::Finished => {
                        let tab = &self.tabs[idx];
                        self.store.record(tab.current_url.as_str(), &tab.title);
                        if active {
                            self.status_text.clear();
                        }
                    }
                    webview::WebViewEvent::Exited => {
                        self.tabs[idx].webview = None;
                        self.tabs[idx].title = "WebKit-вкладка завершилась".to_string();
                        if active {
                            self.status_text =
                                "Процесс WebKit завершился — F5, чтобы перезагрузить".to_string();
                        }
                        break;
                    }
                    _ => {}
                }
            }
        }
        if changed {
            self.mark_full_repaint();
        }
        changed
    }

    /// The active tab's WebKit window and where to show it (below the chrome).
    pub fn embedded_view(&self) -> Option<droplet_platform::EmbeddedView> {
        let handle = self.tabs.get(self.active_tab)?.webview.as_ref()?.handle?;
        let top = CHROME_HEIGHT.min(self.viewport_height);
        let height = self
            .viewport_height
            .saturating_sub(CHROME_HEIGHT + STATUS_BAR_HEIGHT)
            .max(1);
        Some(droplet_platform::EmbeddedView {
            handle,
            x: 0,
            y: top as i32,
            width: self.viewport_width.max(1) as u32,
            height: height as u32,
        })
    }

    /// Applies a finished navigation: history op, URL, page, scroll restore.
    fn commit_navigation(
        &mut self,
        tab_idx: usize,
        pending: nav::PendingNav,
        page: loader::LoadedPage,
    ) {
        let tab = &mut self.tabs[tab_idx];
        let previous = tab.current_url.clone();
        match pending.history {
            nav::HistoryOp::None => {}
            nav::HistoryOp::Push => {
                tab.back_stack.push(previous);
                tab.forward_stack.clear();
            }
            nav::HistoryOp::Back => {
                tab.back_stack.pop();
                tab.forward_stack.push(previous);
            }
            nav::HistoryOp::Forward => {
                tab.forward_stack.pop();
                tab.back_stack.push(previous);
            }
        }
        tab.current_url = page.final_url.clone();
        if tab_idx == self.active_tab {
            self.url_input = page.final_url.as_str().to_string();
        }
        self.apply_loaded_page(tab_idx, page, pending.start);
        if let Some(saved) = pending.restore_scroll {
            if tab_idx == self.active_tab {
                let max_s = self.max_scroll();
                self.tabs[tab_idx].scroll_y = saved.clamp(0.0, max_s);
            } else {
                self.tabs[tab_idx].scroll_y = saved.max(0.0);
            }
        }
        if tab_idx != self.active_tab {
            self.mark_chrome_repaint();
        }
        self.enforce_warm_tab_limit();
    }

    /// Commits every finished background navigation that is still wanted.
    /// Returns `true` if at least one page was applied.
    pub fn poll_navigations(&mut self) -> bool {
        let mut applied = false;
        while let Some(result) = self.nav.try_recv() {
            let Some(tab_idx) = self.tabs.iter().position(|t| {
                t.pending.as_ref().is_some_and(|p| p.nav_id == result.nav_id)
            }) else {
                continue; // superseded, stopped, or its tab was closed
            };
            if let Some(pending) = self.tabs[tab_idx].pending.take() {
                self.commit_navigation(tab_idx, pending, result.page);
                applied = true;
            }
        }
        applied
    }

    /// Whether the active tab has a navigation in flight.
    pub fn is_loading(&self) -> bool {
        self.tabs
            .get(self.active_tab)
            .is_some_and(|t| t.pending.is_some())
    }

    /// Cancels the active tab's navigation; the current page and history stay unchanged.
    pub fn stop(&mut self) {
        let idx = self.active_tab;
        if let Some(view) = self.tabs[idx].webview.as_mut() {
            view.stop();
        }
        if self.tabs[idx].pending.take().is_some() {
            self.url_input = self.tabs[idx].current_url.as_str().to_string();
            self.status_text = self.base_status_text.clone();
            self.mark_chrome_repaint();
        }
    }

    /// Installs the UI wake-up callback and switches navigation to background threads.
    pub fn set_waker(&mut self, waker: droplet_platform::Waker) {
        self.nav.set_waker(waker);
        self.start_deferred();
    }

    fn load_url_into_tab(&mut self, tab_idx: usize, url: Url) {
        self.start_navigation(tab_idx, url, nav::HistoryOp::None, nav::NavRequest::Get, None);
    }

    fn apply_loaded_page(
        &mut self,
        tab_idx: usize,
        page: loader::LoadedPage,
        start: Instant,
    ) {
        if tab_idx == self.active_tab {
            self.hovered_node = None;
            self.active_node = None;
            self.pressed_target = None;
            self.find.dirty = true;
        }

        let mut title = page
            .title_override
            .or_else(|| page.doc.find_title())
            .unwrap_or_else(|| self.tabs[tab_idx].current_url.as_str().to_string());

        let mut doc = page.doc;
        let mut forms = crate::forms::FormState::default();
        let mut dirty = DirtyFlags::clean();
        self.tabs[tab_idx].forms.reset();
        self.tabs[tab_idx].js.execute_scripts_with_dom(
            &page.scripts,
            &mut doc,
            &mut forms,
            &mut dirty,
            &mut title,
        );
        self.tabs[tab_idx].forms = forms;

        let ctx = self.style_context_for_tab(tab_idx);
        let layout_root =
            compute_layout_with_ctx(&doc, &page.stylesheet, &ctx, &page.images);
        let visual = self.tabs[tab_idx].forms.visual(&doc);
        let display_list = build_display_list_with_forms(&layout_root, &visual);
        let content_height = compute_content_height(&layout_root);

        let internal_page = page.final_url.is_about()
            && matches!(page.final_url.path.as_str(), "history" | "bookmarks" | "blank");
        if page.load_ok && !internal_page {
            self.store.record(page.final_url.as_str(), &title);
        }

        let tab = &mut self.tabs[tab_idx];
        tab.title = title;
        tab.doc = Some(doc);
        tab.stylesheet = Some(page.stylesheet);
        tab.uses_interaction = page.uses_interaction;
        tab.images = page.images;
        tab.layout_root = Some(layout_root);
        tab.display_list = Some(display_list);
        tab.scroll_y = 0.0;
        tab.content_height = content_height;
        tab.dirty = DirtyFlags::clean();
        tab.layout_rebuilds = 1;
        tab.display_list_rebuilds = 1;
        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        tab.timing_badge = format!("{elapsed_ms:.1} ms");
        tab.hibernated = false;

        let js_status = tab.js.status_summary();
        if tab_idx == self.active_tab {
            let effective_status = if !page.status_override.is_empty() {
                page.status_override
            } else {
                js_status.unwrap_or_default()
            };
            self.base_status_text = effective_status.clone();
            self.status_text = effective_status;
            self.mark_full_repaint();
        }
    }

    pub fn go_back(&mut self) {
        let idx = self.active_tab;
        if let Some(view) = self.tabs[idx].webview.as_mut() {
            view.back();
            return;
        }
        if let Some(prev) = self.tabs[idx].back_stack.last().cloned() {
            self.start_navigation(idx, prev, nav::HistoryOp::Back, nav::NavRequest::Get, None);
        }
    }

    pub fn go_forward(&mut self) {
        let idx = self.active_tab;
        if let Some(view) = self.tabs[idx].webview.as_mut() {
            view.forward();
            return;
        }
        if let Some(next) = self.tabs[idx].forward_stack.last().cloned() {
            self.start_navigation(idx, next, nav::HistoryOp::Forward, nav::NavRequest::Get, None);
        }
    }

    pub fn reload(&mut self) {
        let idx = self.active_tab;
        if let Some(view) = self.tabs[idx].webview.as_mut() {
            view.reload();
            return;
        }
        let url = self.tabs[idx].current_url.clone();
        self.load_url_into_tab(idx, url);
    }

    pub fn new_tab(&mut self, url: &str) {
        self.active_seq_counter += 1;
        let new_idx = self.tabs.len();
        self.tabs
            .push(TabState::create_tab("New Tab", self.active_seq_counter));
        self.active_tab = new_idx;
        self.hovered_node = None;
        self.active_node = None;
        self.pressed_target = None;
        self.url_focused = false;
        self.load_input_into_tab(new_idx, url, false);
        self.enforce_warm_tab_limit();
    }

    pub fn close_tab(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        if self.tabs.len() == 1 {
            self.active_seq_counter += 1;
            self.tabs[0].reset_for_reuse(self.active_seq_counter);
            self.active_tab = 0;
            self.hovered_node = None;
            self.active_node = None;
            self.pressed_target = None;
            self.url_focused = false;
            self.load_input_into_tab(0, HOME_URL, false);
            return;
        }

        self.tabs.remove(index);
        if index < self.active_tab {
            self.active_tab -= 1;
            self.mark_chrome_repaint();
        } else if index == self.active_tab {
            self.active_tab = index.min(self.tabs.len() - 1);
            self.activate_current_tab();
        } else {
            self.mark_chrome_repaint();
        }
    }

    pub fn switch_tab(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        if index == self.active_tab && !self.tabs[index].hibernated {
            return;
        }
        self.active_tab = index;
        self.activate_current_tab();
    }

    fn activate_current_tab(&mut self) {
        let idx = self.active_tab;
        self.hovered_node = None;
        self.active_node = None;
        self.pressed_target = None;
        self.url_focused = false;
        self.store.dismiss_autocomplete();
        self.find.dirty = true;
        self.active_seq_counter += 1;
        self.tabs[idx].last_active_seq = self.active_seq_counter;
        self.base_status_text.clear();
        self.status_text.clear();

        if self.tabs[idx].hibernated && self.tabs[idx].pending.is_none() {
            let saved_scroll = self.tabs[idx].scroll_y;
            let url = self.tabs[idx].current_url.clone();
            self.start_navigation(
                idx,
                url,
                nav::HistoryOp::None,
                nav::NavRequest::Get,
                Some(saved_scroll),
            );
        } else if let Some(pending) = &self.tabs[idx].pending {
            self.url_input = pending.url.as_str().to_string();
            self.status_text = loading_status(&self.tabs[idx]);
        } else {
            self.url_input = self.tabs[idx].current_url.as_str().to_string();
            let status = self.tabs[idx].js.status_summary().unwrap_or_default();
            self.base_status_text = status.clone();
            self.status_text = status;
        }

        self.enforce_warm_tab_limit();
        self.mark_full_repaint();
    }

    pub fn hibernate_tab(&mut self, index: usize) {
        let Some(tab) = self.tabs.get_mut(index) else {
            return;
        };
        tab.clear_warm_payload();

        if index == self.active_tab {
            self.hovered_node = None;
            self.active_node = None;
            self.pressed_target = None;
            self.find.dirty = true;
            self.mark_full_repaint();
        }
    }

    fn enforce_warm_tab_limit(&mut self) {
        loop {
            let warm_count = self.tabs.iter().filter(|t| !t.hibernated).count();
            if warm_count <= MAX_WARM_TABS {
                break;
            }
            let victim = self
                .tabs
                .iter()
                .enumerate()
                .filter(|(idx, t)| *idx != self.active_tab && !t.hibernated)
                .min_by_key(|(_, t)| t.last_active_seq)
                .map(|(idx, _)| idx);
            let Some(victim_idx) = victim else {
                break;
            };
            self.hibernate_tab(victim_idx);
        }
    }

    pub fn resize(&mut self, width: usize, height: usize) {
        let w = width.max(1);
        let h = height.max(1);
        if w == self.viewport_width && h == self.viewport_height {
            return;
        }
        self.viewport_width = w;
        self.viewport_height = h;

        let page_vh = self.page_viewport_height();
        for idx in 0..self.tabs.len() {
            if self.tabs[idx].hibernated {
                continue;
            }
            let ctx = self.style_context_for_tab(idx);
            relayout_tab_state(&mut self.tabs[idx], &ctx, page_vh);
        }

        self.find.dirty = true;
        self.mark_full_repaint();
    }

    pub fn page_viewport_height(&self) -> f32 {
        self.viewport_height
            .saturating_sub(CHROME_HEIGHT + STATUS_BAR_HEIGHT) as f32
    }

    fn max_scroll(&self) -> f32 {
        let content_h = self
            .tabs
            .get(self.active_tab)
            .map(|t| t.content_height)
            .unwrap_or(0.0);
        (content_h - self.page_viewport_height()).max(0.0)
    }

    pub fn scroll(&mut self, delta_y: f32) {
        if !delta_y.is_finite() {
            return;
        }
        let max_scroll = self.max_scroll();
        let tab = &mut self.tabs[self.active_tab];
        let next = (tab.scroll_y + delta_y).clamp(0.0, max_scroll);
        if (next - tab.scroll_y).abs() > f32::EPSILON {
            tab.scroll_y = next;
            self.mark_viewport_repaint();
        }
    }

    fn style_context_for_tab(&self, tab_idx: usize) -> StyleContext {
        let (hovered_node, active_node) = if tab_idx == self.active_tab {
            (self.hovered_node, self.active_node)
        } else {
            (None, None)
        };
        let focused_node = self
            .tabs
            .get(tab_idx)
            .and_then(|tab| tab.forms.focused)
            .filter(|node| {
                self.tabs
                    .get(tab_idx)
                    .and_then(|tab| tab.doc.as_ref())
                    .and_then(|doc| doc.get(*node))
                    .is_some()
            });
        StyleContext {
            viewport_width: self.viewport_width as f32,
            viewport_height: self.page_viewport_height().max(1.0),
            hovered_node,
            active_node,
            focused_node,
        }
    }

    fn restyle_active_tab(&mut self) {
        let ctx = self.style_context_for_tab(self.active_tab);
        let page_vh = self.page_viewport_height();
        let tab = &mut self.tabs[self.active_tab];
        if !tab.hibernated {
            relayout_tab_state(tab, &ctx, page_vh);
        }
    }

    fn update_interaction_state(&mut self, hovered: Option<NodeId>, active: Option<NodeId>) {
        let prev_state = (self.hovered_node, self.active_node);
        self.hovered_node = hovered;
        self.active_node = active;
        if (self.hovered_node, self.active_node) != prev_state {
            if self.tabs[self.active_tab].uses_interaction {
                self.restyle_active_tab();
            }
            if self.tabs[self.active_tab].uses_interaction || self.inspector_enabled {
                self.mark_viewport_repaint();
            }
        }
    }

    fn update_hover_status(&mut self, hit_href: Option<&str>) {
        let new_status = match hit_href {
            Some(href) => href.to_string(),
            None => self.base_status_text.clone(),
        };
        if new_status != self.status_text {
            self.status_text = new_status;
            self.mark_status_repaint();
        }
    }

    fn chrome_target_at(&self, x: f32, y: f32) -> Option<ChromeTarget> {
        let hitboxes = chrome_hitboxes(self.viewport_width, self.tabs.len());
        for (i, close_rect) in hitboxes.tab_close_rects.iter().enumerate() {
            if close_rect.width > 0.0 && close_rect.contains(x, y) {
                return Some(ChromeTarget::TabClose(i));
            }
        }
        for (i, tab_rect) in hitboxes.tab_rects.iter().enumerate() {
            if tab_rect.contains(x, y) {
                return Some(ChromeTarget::Tab(i));
            }
        }
        if hitboxes.new_tab_btn.contains(x, y) {
            return Some(ChromeTarget::NewTabBtn);
        }
        if hitboxes.back_btn.contains(x, y) {
            return Some(ChromeTarget::BackBtn);
        }
        if hitboxes.forward_btn.contains(x, y) {
            return Some(ChromeTarget::ForwardBtn);
        }
        if hitboxes.reload_btn.contains(x, y) {
            return Some(ChromeTarget::ReloadBtn);
        }
        if hitboxes.home_btn.contains(x, y) {
            return Some(ChromeTarget::HomeBtn);
        }
        if hitboxes.address_bar.contains(x, y) {
            return Some(ChromeTarget::AddressBar);
        }
        None
    }

    fn activate_chrome_target(&mut self, target: ChromeTarget) {
        match target {
            ChromeTarget::TabClose(i) => {
                self.url_focused = false;
                self.close_tab(i);
            }
            ChromeTarget::Tab(i) => {
                if self.url_focused {
                    self.url_focused = false;
                    self.mark_chrome_repaint();
                }
                self.switch_tab(i);
            }
            ChromeTarget::NewTabBtn => {
                self.url_focused = false;
                self.new_tab(HOME_URL);
            }
            ChromeTarget::BackBtn => {
                if self.url_focused {
                    self.url_focused = false;
                    self.mark_chrome_repaint();
                }
                self.go_back();
            }
            ChromeTarget::ForwardBtn => {
                if self.url_focused {
                    self.url_focused = false;
                    self.mark_chrome_repaint();
                }
                self.go_forward();
            }
            ChromeTarget::ReloadBtn => {
                if self.url_focused {
                    self.url_focused = false;
                    self.mark_chrome_repaint();
                }
                self.reload();
            }
            ChromeTarget::HomeBtn => {
                if self.url_focused {
                    self.url_focused = false;
                    self.mark_chrome_repaint();
                }
                self.navigate(HOME_URL);
            }
            ChromeTarget::AddressBar => {
                self.store.dismiss_autocomplete();
                if !self.url_focused {
                    self.url_focused = true;
                    self.mark_chrome_repaint();
                }
            }
        }
    }

    fn hit_test_viewport(&self, x: f32, y: f32) -> (Option<NodeId>, Option<String>) {
        let vp_top = CHROME_HEIGHT as f32;
        let vp_bottom = self.viewport_height.saturating_sub(STATUS_BAR_HEIGHT) as f32;
        if y < vp_top || y >= vp_bottom {
            return (None, None);
        }
        let tab = &self.tabs[self.active_tab];
        let Some(root) = tab.layout_root.as_ref() else {
            return (None, None);
        };
        let doc_x = x;
        let doc_y = y - vp_top + tab.scroll_y;
        let node = droplet_layout::hit_test_node(root, doc_x, doc_y);
        let href = droplet_layout::hit_test_href(root, doc_x, doc_y).map(str::to_string);
        (node, href)
    }

    fn focused_control(&self) -> Option<NodeId> {
        let tab = self.tabs.get(self.active_tab)?;
        let node = tab.forms.focused?;
        forms::is_focusable(tab.doc.as_ref()?, node).then_some(node)
    }

    fn doc_point(&self, x: f32, y: f32) -> Option<(f32, f32)> {
        let tab = self.tabs.get(self.active_tab)?;
        let vp_top = CHROME_HEIGHT as f32;
        let vp_bottom = self.viewport_height.saturating_sub(STATUS_BAR_HEIGHT) as f32;
        (y >= vp_top && y < vp_bottom).then_some((x, y - vp_top + tab.scroll_y))
    }

    /// Flushes only the dirty layers on the active tab (no-op when clean).
    pub fn flush_active_dirty(&mut self) {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return;
        };
        let dirty = tab.dirty.take();
        if dirty.is_clean() {
            return;
        }
        if dirty.title_changed {
            if let Some(next_title) = tab.doc.as_ref().and_then(Document::find_title) {
                tab.title = next_title;
            }
            self.mark_chrome_repaint();
        }
        if dirty.style_and_layout {
            self.restyle_active_tab();
            self.find.dirty = true;
            self.mark_viewport_repaint();
        } else if dirty.paint {
            self.refresh_display_list();
        }
    }

    /// Rebuilds only the display list (caret/selection changes need no layout).
    fn refresh_display_list(&mut self) {
        let tab = &mut self.tabs[self.active_tab];
        let visual = match (tab.doc.as_ref(), tab.layout_root.as_ref()) {
            (Some(doc), Some(_)) => tab.forms.visual(doc),
            _ => FormVisualState::default(),
        };
        if let Some(root) = tab.layout_root.as_ref() {
            tab.display_list = Some(build_display_list_with_forms(root, &visual));
            tab.display_list_rebuilds += 1;
        }
        tab.dirty.paint = false;
        self.mark_viewport_repaint();
    }

    fn refresh_active_layout(&mut self) {
        self.restyle_active_tab();
        self.mark_viewport_repaint();
    }

    fn set_form_focus(&mut self, focus: Option<NodeId>) {
        let idx = self.active_tab;
        let changed = self.tabs[idx].forms.focused != focus;
        let caret = focus
            .and_then(|node| {
                self.tabs[idx].doc.as_ref().and_then(|doc| {
                    forms::is_text_like(doc, node).then(|| forms::value_of(doc, node).len())
                })
            })
            .unwrap_or(0);
        {
            let state = &mut self.tabs[idx].forms;
            state.focused = focus;
            state.caret = caret;
            state.anchor = None;
            state.control_dragging = false;
            state.page_drag_anchor = None;
        }
        if changed {
            self.refresh_active_layout();
        } else {
            self.refresh_display_list();
        }
    }

    fn focus_control_clicked(&mut self, node: NodeId, x: f32, doc_y: f32) {
        let idx = self.active_tab;
        let kind = self.tabs[idx]
            .doc
            .as_ref()
            .and_then(|doc| forms::control_kind(doc, node));
        self.set_form_focus(Some(node));
        if matches!(
            kind,
            Some(
                droplet_layout::ControlKind::TextInput
                    | droplet_layout::ControlKind::PasswordInput
                    | droplet_layout::ControlKind::Textarea
            )
        ) {
            let caret = self.tabs[idx]
                .layout_root
                .as_ref()
                .and_then(|root| forms::control_char_at(root, node, x, doc_y))
                .map(|chars| {
                    let value = self.tabs[idx]
                        .doc
                        .as_ref()
                        .map(|doc| forms::value_of(doc, node))
                        .unwrap_or_default();
                    forms::byte_at_char(&value, chars)
                })
                .unwrap_or(0);
            let state = &mut self.tabs[idx].forms;
            state.caret = caret;
            state.anchor = Some(caret);
            state.control_dragging = true;
            state.page_selection = None;
            self.refresh_display_list();
        } else {
            let state = &mut self.tabs[idx].forms;
            if state.page_selection.is_some() {
                state.page_selection = None;
                self.refresh_display_list();
            }
        }
    }

    fn control_char_under_pointer(&self, node: NodeId, x: f32, doc_y: f32) -> Option<usize> {
        let root = self.tabs[self.active_tab].layout_root.as_ref()?;
        forms::control_char_at(root, node, x, doc_y)
    }

    fn update_control_drag(&mut self, x: f32, doc_y: f32) {
        let idx = self.active_tab;
        let Some(node) = self.tabs[idx].forms.focused else {
            return;
        };
        if !self.tabs[idx].forms.control_dragging {
            return;
        }
        let Some(chars) = self.control_char_under_pointer(node, x, doc_y) else {
            return;
        };
        let value = self.tabs[idx]
            .doc
            .as_ref()
            .map(|doc| forms::value_of(doc, node))
            .unwrap_or_default();
        let byte = forms::byte_at_char(&value, chars);
        self.tabs[idx].forms.caret = byte;
        self.refresh_display_list();
    }

    fn update_page_drag(&mut self, x: f32, doc_y: f32) {
        let idx = self.active_tab;
        let Some(anchor) = self.tabs[idx].forms.page_drag_anchor else {
            return;
        };
        let Some(root) = self.tabs[idx].layout_root.as_ref() else {
            return;
        };
        let Some(focus) = forms::text_position_at(root, x, doc_y) else {
            return;
        };
        self.tabs[idx].forms.page_selection = Some(droplet_paint::PageSelection { anchor, focus });
        self.refresh_display_list();
    }

    fn is_submit_control(&self, node: NodeId) -> bool {
        let Some(doc) = self.tabs[self.active_tab].doc.as_ref() else {
            return false;
        };
        let Some(elem) = doc.element(node) else {
            return false;
        };
        if elem.tag_name.eq_ignore_ascii_case("input") {
            return forms::control_kind(doc, node) == Some(droplet_layout::ControlKind::Submit);
        }
        let ty = elem.attr("type").unwrap_or("submit");
        ty.eq_ignore_ascii_case("submit")
    }

    fn submit_form(&mut self, trigger: NodeId) {
        let idx = self.active_tab;
        let Some(doc) = self.tabs[idx].doc.as_ref() else {
            return;
        };
        let form_target = forms::form_ancestor(doc, trigger).unwrap_or(trigger);
        let outcome = self.dispatch_dom_event("submit", form_target, "");
        if outcome.default_prevented {
            return;
        }
        let Some(doc) = self.tabs[idx].doc.as_ref() else {
            return;
        };
        let base = self.tabs[idx].current_url.clone();
        let Some(submission) = forms::prepare_submission(doc, trigger, &base) else {
            return;
        };
        let request = match submission.method {
            forms::FormMethod::Get => nav::NavRequest::Get,
            forms::FormMethod::Post => nav::NavRequest::Post(submission.body),
        };
        self.start_navigation(idx, submission.url, nav::HistoryOp::Push, request, None);
    }

    fn activate_control(&mut self, node: NodeId) {
        let kind = self.tabs[self.active_tab]
            .doc
            .as_ref()
            .and_then(|doc| forms::control_kind(doc, node));
        match kind {
            Some(droplet_layout::ControlKind::Checkbox)
            | Some(droplet_layout::ControlKind::Radio) => {
                let toggled = self.tabs[self.active_tab]
                    .doc
                    .as_mut()
                    .is_some_and(|doc| forms::toggle_checked(doc, node));
                self.refresh_active_layout();
                if toggled {
                    self.dispatch_dom_event("input", node, "");
                    self.dispatch_dom_event("change", node, "");
                }
            }
            Some(droplet_layout::ControlKind::Select) => {
                let cycled = self.tabs[self.active_tab]
                    .doc
                    .as_mut()
                    .is_some_and(|doc| forms::cycle_select(doc, node, 1));
                self.refresh_active_layout();
                if cycled {
                    self.dispatch_dom_event("input", node, "");
                    self.dispatch_dom_event("change", node, "");
                }
            }
            Some(droplet_layout::ControlKind::Submit | droplet_layout::ControlKind::Button)
                if self.is_submit_control(node) =>
            {
                self.submit_form(node);
            }
            _ => {}
        }
    }

    fn insert_focused_text(&mut self, text: &str) -> bool {
        let idx = self.active_tab;
        let Some(node) = self.focused_control() else {
            return false;
        };
        if !self.tabs[idx]
            .doc
            .as_ref()
            .is_some_and(|doc| forms::is_text_like(doc, node))
        {
            return false;
        }
        let selection = self.tabs[idx].forms.control_selection();
        let mut caret = self.tabs[idx].forms.caret;
        let changed = self.tabs[idx]
            .doc
            .as_mut()
            .is_some_and(|doc| forms::insert_text(doc, node, text, &mut caret, selection));
        if changed {
            let state = &mut self.tabs[idx].forms;
            state.caret = caret;
            state.anchor = None;
            self.refresh_active_layout();
            self.dispatch_dom_event("input", node, "");
        }
        changed
    }

    fn copy_current_selection(&mut self) -> bool {
        let idx = self.active_tab;
        if let Some(node) = self.focused_control() {
            if let Some((start, end)) = self.tabs[idx].forms.control_selection() {
                if let Some(doc) = self.tabs[idx].doc.as_ref() {
                    let value = forms::value_of(doc, node);
                    if let Some(text) = forms::selected_slice(&value, start, end) {
                        return droplet_platform::clipboard::set_text(text);
                    }
                }
            }
        }
        if let Some(selection) = self.tabs[idx].forms.page_selection {
            if let Some(root) = self.tabs[idx].layout_root.as_ref() {
                let text = forms::page_selection_text(root, &selection);
                if !text.is_empty() {
                    return droplet_platform::clipboard::set_text(&text);
                }
            }
        }
        false
    }

    fn paste_into_focused(&mut self) -> bool {
        let Some(text) = droplet_platform::clipboard::get_text() else {
            return false;
        };
        let normalized = self
            .focused_control()
            .and_then(|node| self.tabs[self.active_tab].doc.as_ref().map(|doc| (doc, node)))
            .map(|(doc, node)| {
                if forms::control_kind(doc, node) == Some(droplet_layout::ControlKind::Textarea) {
                    forms::sanitize_clipboard_text(&text)
                } else {
                    forms::single_line_clipboard_text(&text)
                }
            });
        match normalized {
            Some(text) => self.insert_focused_text(&text),
            None => false,
        }
    }

    pub fn handle_mouse_move(&mut self, x: f32, y: f32) {
        let idx = self.active_tab;
        if self.tabs[idx].forms.control_dragging {
            if let Some((doc_x, doc_y)) = self.doc_point(x, y) {
                self.update_control_drag(doc_x, doc_y);
            }
        } else if self.tabs[idx].forms.page_drag_anchor.is_some() {
            if let Some((doc_x, doc_y)) = self.doc_point(x, y) {
                self.update_page_drag(doc_x, doc_y);
            }
        }
        let (new_hovered, hit_href) = self.hit_test_viewport(x, y);
        self.update_interaction_state(new_hovered, self.active_node);
        self.update_hover_status(hit_href.as_deref());
    }

    pub fn handle_mouse_down(&mut self, x: f32, y: f32) {
        let idx = self.active_tab;
        self.tabs[idx].forms.control_dragging = false;
        self.tabs[idx].forms.page_drag_anchor = None;

        if let Some(target) = self.chrome_target_at(x, y) {
            self.pressed_target = Some(PressedTarget::Chrome(target));
            let should_focus = target == ChromeTarget::AddressBar;
            if should_focus {
                self.store.dismiss_autocomplete();
            }
            if self.url_focused != should_focus {
                self.url_focused = should_focus;
                self.mark_chrome_repaint();
            }
            self.update_interaction_state(None, None);
            self.update_hover_status(None);
            return;
        }

        if self.url_focused {
            self.url_focused = false;
            self.mark_chrome_repaint();
        }

        if self.find.open
            && droplet_paint::find_bar_rect(self.viewport_width, CHROME_HEIGHT).contains(x, y)
        {
            self.pressed_target = None;
            self.update_interaction_state(None, None);
            return;
        }

        let Some((doc_x, doc_y)) = self.doc_point(x, y) else {
            self.pressed_target = None;
            self.update_interaction_state(None, None);
            return;
        };

        let (hit_node, hit_href) = self.hit_test_viewport(x, y);
        let control_hit = hit_node.filter(|&node| {
            self.tabs[idx]
                .doc
                .as_ref()
                .is_some_and(|doc| forms::control_kind(doc, node).is_some())
        });
        if let Some(node) = control_hit {
            let focusable = self.tabs[idx]
                .doc
                .as_ref()
                .is_some_and(|doc| forms::is_focusable(doc, node));
            if focusable {
                self.pressed_target = Some(PressedTarget::Viewport {
                    href: hit_href,
                    control: Some(node),
                    node: hit_node,
                });
                self.focus_control_clicked(node, doc_x, doc_y);
            } else {
                self.pressed_target = Some(PressedTarget::Viewport {
                    href: None,
                    control: None,
                    node: None,
                });
            }
            self.update_interaction_state(hit_node, hit_node);
            return;
        }

        if self.tabs[idx].forms.focused.is_some() {
            self.set_form_focus(None);
        }
        let anchor = self.tabs[idx]
            .layout_root
            .as_ref()
            .and_then(|root| forms::text_position_at(root, doc_x, doc_y));
        self.tabs[idx].forms.page_drag_anchor = anchor;
        if self.tabs[idx].forms.page_selection.is_some() {
            self.tabs[idx].forms.page_selection = None;
            self.refresh_display_list();
        }
        self.pressed_target = Some(PressedTarget::Viewport {
            href: hit_href,
            control: None,
            node: hit_node,
        });
        self.update_interaction_state(hit_node, hit_node);
    }

    pub fn handle_mouse_up(&mut self, x: f32, y: f32) {
        let pressed = self.pressed_target.take();
        let idx = self.active_tab;
        let was_control_dragging = self.tabs[idx].forms.control_dragging;
        self.tabs[idx].forms.control_dragging = false;
        self.tabs[idx].forms.page_drag_anchor = None;
        if was_control_dragging && self.tabs[idx].forms.control_selection().is_none() {
            self.tabs[idx].forms.anchor = None;
        }

        let (new_hovered, hit_href) = self.hit_test_viewport(x, y);

        self.update_interaction_state(new_hovered, None);
        self.update_hover_status(hit_href.as_deref());

        if self.find.open
            && droplet_paint::find_bar_rect(self.viewport_width, CHROME_HEIGHT).contains(x, y)
        {
            return;
        }

        match pressed {
            Some(PressedTarget::Chrome(target)) => {
                if self.chrome_target_at(x, y) == Some(target) {
                    self.activate_chrome_target(target);
                }
            }
            Some(PressedTarget::Viewport {
                control: Some(node),
                ..
            }) if new_hovered == Some(node) => {
                let outcome = self.dispatch_dom_event("click", node, "");
                if outcome.default_prevented {
                    return;
                }
                let kind = self.tabs[idx]
                    .doc
                    .as_ref()
                    .and_then(|doc| forms::control_kind(doc, node));
                if matches!(
                    kind,
                    Some(
                        droplet_layout::ControlKind::Checkbox
                            | droplet_layout::ControlKind::Radio
                            | droplet_layout::ControlKind::Select
                            | droplet_layout::ControlKind::Submit
                            | droplet_layout::ControlKind::Button
                    )
                ) {
                    self.activate_control(node);
                }
            }
            Some(PressedTarget::Viewport {
                href,
                control: None,
                node,
            }) if self.tabs[idx].forms.page_selection.is_none() => {
                let click_target = new_hovered.or(node);
                let outcome = click_target
                    .map(|target| self.dispatch_dom_event("click", target, ""))
                    .unwrap_or_default();
                if !outcome.default_prevented && hit_href.as_deref() == href.as_deref() {
                    if let Some(down_href) = href {
                        self.navigate(&down_href);
                    }
                }
            }
            _ => {}
        }
    }

    pub fn handle_click(&mut self, x: f32, y: f32) {
        self.handle_mouse_down(x, y);
        self.handle_mouse_up(x, y);
    }

    pub fn handle_text_input(&mut self, ch: char) {
        if ch.is_control() {
            return;
        }
        if self.url_focused {
            self.url_input.push(ch);
            let typed = self.url_input.clone();
            self.store.refresh_autocomplete(&typed);
            self.mark_chrome_repaint();
            return;
        }
        if self.find.open {
            self.find.push_char(ch);
            self.mark_full_repaint();
            return;
        }
        if let Some(node) = self.focused_control() {
            let is_text = self.tabs[self.active_tab]
                .doc
                .as_ref()
                .is_some_and(|doc| forms::is_text_like(doc, node));
            let mut buf = [0u8; 4];
            let key_str = ch.encode_utf8(&mut buf).to_string();
            let keydown = self.dispatch_dom_event("keydown", node, &key_str);
            if !keydown.default_prevented {
                if is_text {
                    self.insert_focused_text(&key_str);
                } else if ch == ' ' {
                    let click = self.dispatch_dom_event("click", node, " ");
                    if !click.default_prevented {
                        self.activate_control(node);
                    }
                }
            }
            self.dispatch_dom_event("keyup", node, &key_str);
            return;
        }
        if ch == ' ' {
            self.handle_key_navigation(NavKey::PageDown);
        }
    }

    pub fn handle_backspace(&mut self) {
        if self.url_focused {
            if self.url_input.pop().is_some() {
                let typed = self.url_input.clone();
                self.store.refresh_autocomplete(&typed);
                self.mark_chrome_repaint();
            }
            return;
        }
        if self.find.open {
            if !self.find.query.is_empty() {
                self.find.pop_char();
                self.mark_full_repaint();
            }
            return;
        }
        let idx = self.active_tab;
        let Some(node) = self.focused_control() else {
            return;
        };
        let keydown = self.dispatch_dom_event("keydown", node, "Backspace");
        if keydown.default_prevented {
            self.dispatch_dom_event("keyup", node, "Backspace");
            return;
        }
        if !self.tabs[idx]
            .doc
            .as_ref()
            .is_some_and(|doc| forms::is_text_like(doc, node))
        {
            self.dispatch_dom_event("keyup", node, "Backspace");
            return;
        }
        let selection = self.tabs[idx].forms.control_selection();
        let mut caret = self.tabs[idx].forms.caret;
        let changed = self.tabs[idx]
            .doc
            .as_mut()
            .is_some_and(|doc| forms::delete_backward(doc, node, &mut caret, selection));
        if changed {
            let state = &mut self.tabs[idx].forms;
            state.caret = caret;
            state.anchor = None;
            self.refresh_active_layout();
            self.dispatch_dom_event("input", node, "");
        }
        self.dispatch_dom_event("keyup", node, "Backspace");
    }

    pub fn handle_delete(&mut self) {
        if self.url_focused {
            return;
        }
        let idx = self.active_tab;
        let Some(node) = self.focused_control() else {
            return;
        };
        let keydown = self.dispatch_dom_event("keydown", node, "Delete");
        if keydown.default_prevented {
            self.dispatch_dom_event("keyup", node, "Delete");
            return;
        }
        if !self.tabs[idx]
            .doc
            .as_ref()
            .is_some_and(|doc| forms::is_text_like(doc, node))
        {
            self.dispatch_dom_event("keyup", node, "Delete");
            return;
        }
        let selection = self.tabs[idx].forms.control_selection();
        let mut caret = self.tabs[idx].forms.caret;
        let changed = self.tabs[idx]
            .doc
            .as_mut()
            .is_some_and(|doc| forms::delete_forward(doc, node, &mut caret, selection));
        if changed {
            let state = &mut self.tabs[idx].forms;
            state.caret = caret;
            state.anchor = None;
            self.refresh_active_layout();
            self.dispatch_dom_event("input", node, "");
        }
        self.dispatch_dom_event("keyup", node, "Delete");
    }

    pub fn handle_tab_focus(&mut self, backwards: bool) {
        if self.url_focused {
            self.url_focused = false;
            self.mark_chrome_repaint();
        }
        let idx = self.active_tab;
        let current = self.tabs[idx].forms.focused;
        let next = self.tabs[idx]
            .doc
            .as_ref()
            .and_then(|doc| forms::next_focus(doc, current, backwards));
        self.set_form_focus(next);
    }

    pub fn handle_caret(&mut self, movement: CaretMove, extend: bool) {
        if self.url_focused {
            return;
        }
        let idx = self.active_tab;
        if let Some(node) = self.focused_control() {
            let key_name = match movement {
                CaretMove::Left => "ArrowLeft",
                CaretMove::Right => "ArrowRight",
                CaretMove::Up => "ArrowUp",
                CaretMove::Down => "ArrowDown",
                CaretMove::Home => "Home",
                CaretMove::End => "End",
            };
            let keydown = self.dispatch_dom_event("keydown", node, key_name);
            if keydown.default_prevented {
                self.dispatch_dom_event("keyup", node, key_name);
                return;
            }
            let kind = self.tabs[idx]
                .doc
                .as_ref()
                .and_then(|doc| forms::control_kind(doc, node));
            if kind.is_some_and(|k| k.is_text_like()) {
                let value = self.tabs[idx]
                    .doc
                    .as_ref()
                    .map(|doc| forms::value_of(doc, node))
                    .unwrap_or_default();
                let mov = match movement {
                    CaretMove::Left => forms::CaretMovement::Left,
                    CaretMove::Right => forms::CaretMovement::Right,
                    CaretMove::Up => forms::CaretMovement::Up,
                    CaretMove::Down => forms::CaretMovement::Down,
                    CaretMove::Home => forms::CaretMovement::Home,
                    CaretMove::End => forms::CaretMovement::End,
                };
                let state = &mut self.tabs[idx].forms;
                let old_caret = state.caret.min(value.len());
                let new_caret = forms::move_caret(&value, old_caret, mov);
                if extend {
                    if state.anchor.is_none() {
                        state.anchor = Some(old_caret);
                    }
                    state.caret = new_caret;
                } else {
                    state.caret = new_caret;
                    state.anchor = None;
                }
                self.refresh_display_list();
                self.dispatch_dom_event("keyup", node, key_name);
                return;
            }
            if kind == Some(droplet_layout::ControlKind::Select) {
                let delta = match movement {
                    CaretMove::Left | CaretMove::Up => -1,
                    CaretMove::Right | CaretMove::Down => 1,
                    CaretMove::Home | CaretMove::End => 0,
                };
                if delta != 0
                    && self.tabs[idx]
                        .doc
                        .as_mut()
                        .is_some_and(|doc| forms::cycle_select(doc, node, delta))
                {
                    self.refresh_active_layout();
                    self.dispatch_dom_event("input", node, "");
                    self.dispatch_dom_event("change", node, "");
                }
                self.dispatch_dom_event("keyup", node, key_name);
                return;
            }
            self.dispatch_dom_event("keyup", node, key_name);
            return;
        }
        match movement {
            CaretMove::Up => self.handle_key_navigation(NavKey::Up),
            CaretMove::Down => self.handle_key_navigation(NavKey::Down),
            CaretMove::Home => self.handle_key_navigation(NavKey::Home),
            CaretMove::End => self.handle_key_navigation(NavKey::End),
            CaretMove::Left | CaretMove::Right => {}
        }
    }

    pub fn handle_activate(&mut self) {
        if self.url_focused {
            self.handle_submit_url();
            return;
        }
        let Some(node) = self.focused_control() else {
            return;
        };
        let prev_back_len = self.tabs[self.active_tab].back_stack.len();
        let keydown = self.dispatch_dom_event("keydown", node, "Enter");
        if keydown.default_prevented {
            self.dispatch_dom_event("keyup", node, "Enter");
            return;
        }
        let kind = self.tabs[self.active_tab]
            .doc
            .as_ref()
            .and_then(|doc| forms::control_kind(doc, node));
        match kind {
            Some(droplet_layout::ControlKind::Textarea) => {
                self.insert_focused_text("\n");
            }
            Some(droplet_layout::ControlKind::TextInput | droplet_layout::ControlKind::PasswordInput) => {
                self.submit_form(node);
            }
            _ => {
                let click = self.dispatch_dom_event("click", node, "Enter");
                if !click.default_prevented {
                    self.activate_control(node);
                }
            }
        }
        if self.tabs[self.active_tab].back_stack.len() == prev_back_len {
            self.dispatch_dom_event("keyup", node, "Enter");
        }
    }

    pub fn handle_select_all(&mut self) {
        if self.url_focused {
            return;
        }
        let idx = self.active_tab;
        if let Some(node) = self.focused_control() {
            let is_text = self.tabs[idx]
                .doc
                .as_ref()
                .is_some_and(|doc| forms::is_text_like(doc, node));
            if is_text {
                let len = self.tabs[idx]
                    .doc
                    .as_ref()
                    .map(|doc| forms::value_of(doc, node).len())
                    .unwrap_or(0);
                let state = &mut self.tabs[idx].forms;
                state.anchor = Some(0);
                state.caret = len;
                state.page_selection = None;
                self.refresh_display_list();
                return;
            }
        }
        let sel = self.tabs[idx]
            .layout_root
            .as_ref()
            .and_then(forms::select_all_page);
        if sel.is_some() {
            self.tabs[idx].forms.page_selection = sel;
            self.refresh_display_list();
        }
    }

    pub fn handle_copy(&mut self) {
        self.copy_current_selection();
    }

    pub fn handle_cut(&mut self) {
        if self.url_focused {
            return;
        }
        self.copy_current_selection();
        let idx = self.active_tab;
        let Some(node) = self.focused_control() else {
            return;
        };
        if !self.tabs[idx]
            .doc
            .as_ref()
            .is_some_and(|doc| forms::is_text_like(doc, node))
        {
            return;
        }
        let Some((start, end)) = self.tabs[idx].forms.control_selection() else {
            return;
        };
        let mut caret = self.tabs[idx].forms.caret;
        let changed = self.tabs[idx]
            .doc
            .as_mut()
            .is_some_and(|doc| forms::delete_range(doc, node, start, end, &mut caret));
        if changed {
            let state = &mut self.tabs[idx].forms;
            state.caret = caret;
            state.anchor = None;
            self.refresh_active_layout();
            self.dispatch_dom_event("input", node, "");
            self.dispatch_dom_event("change", node, "");
        }
    }

    pub fn handle_paste(&mut self) {
        if self.url_focused {
            return;
        }
        self.paste_into_focused();
    }

    pub fn handle_submit_url(&mut self) {
        self.store.dismiss_autocomplete();
        let target = self.url_input.trim().to_string();
        self.url_focused = false;
        if target.is_empty() {
            self.url_input = self.active_url_str().to_string();
            self.mark_chrome_repaint();
        } else {
            self.navigate(&target);
        }
    }

    pub fn handle_key_navigation(&mut self, key: NavKey) {
        match key {
            NavKey::Up => {
                if self.url_focused {
                    self.cycle_autocomplete(false);
                } else {
                    self.scroll(-48.0);
                }
            }
            NavKey::Down => {
                if self.url_focused {
                    self.cycle_autocomplete(true);
                } else {
                    self.scroll(48.0);
                }
            }
            NavKey::PageUp => {
                let step = (self.page_viewport_height() * 0.85).max(48.0);
                self.scroll(-step);
            }
            NavKey::PageDown => {
                let step = (self.page_viewport_height() * 0.85).max(48.0);
                self.scroll(step);
            }
            NavKey::Home => {
                let sy = self.tabs[self.active_tab].scroll_y;
                self.scroll(-sy);
            }
            NavKey::End => {
                let max_s = self.max_scroll();
                let sy = self.tabs[self.active_tab].scroll_y;
                self.scroll(max_s - sy);
            }
            NavKey::Back => self.go_back(),
            NavKey::Forward => self.go_forward(),
            NavKey::Reload => self.reload(),
            NavKey::FocusUrl => {
                if !self.url_focused {
                    self.store.dismiss_autocomplete();
                    self.url_focused = true;
                    self.mark_chrome_repaint();
                }
            }
            NavKey::Escape => {
                if self.is_loading() && !self.find.open {
                    // Esc first stops a page load (also closes the address-bar edit).
                    self.url_focused = false;
                    self.store.dismiss_autocomplete();
                    self.stop();
                } else if self.url_focused {
                    self.store.dismiss_autocomplete();
                    self.url_focused = false;
                    self.url_input = self.active_url_str().to_string();
                    self.mark_chrome_repaint();
                } else if self.find.open {
                    self.find.hide();
                    self.mark_full_repaint();
                } else {
                    let idx = self.active_tab;
                    if self.tabs[idx].forms.focused.is_some() {
                        self.set_form_focus(None);
                    }
                    if self.tabs[idx].forms.page_selection.is_some() {
                        self.tabs[idx].forms.page_selection = None;
                        self.refresh_display_list();
                    }
                }
            }
            NavKey::NewTab => self.new_tab(HOME_URL),
            NavKey::CloseTab => self.close_tab(self.active_tab),
            NavKey::NextTab => {
                if self.tabs.len() > 1 {
                    let next = (self.active_tab + 1) % self.tabs.len();
                    self.switch_tab(next);
                }
            }
            NavKey::PrevTab => {
                if self.tabs.len() > 1 {
                    let prev = if self.active_tab == 0 {
                        self.tabs.len() - 1
                    } else {
                        self.active_tab - 1
                    };
                    self.switch_tab(prev);
                }
            }
            NavKey::SwitchTab(index) => self.switch_tab(index),
            NavKey::GoHome => self.navigate(HOME_URL),
            NavKey::ToggleBookmark => self.toggle_current_bookmark(),
            NavKey::Find => self.open_find(),
            NavKey::FindNext => self.step_find(true),
            NavKey::FindPrev => self.step_find(false),
            NavKey::AcceptCompletion => {
                if self.url_focused {
                    self.accept_autocomplete();
                }
            }
            NavKey::ToggleInspector => {
                self.inspector_enabled = !self.inspector_enabled;
                self.mark_full_repaint();
            }
        }
    }

    fn open_find(&mut self) {
        self.url_focused = false;
        self.store.dismiss_autocomplete();
        self.find.show();
        self.ensure_find_matches();
        self.mark_full_repaint();
    }

    fn ensure_find_matches(&mut self) {
        if !self.find.open || !self.find.dirty {
            return;
        }
        let root = self
            .tabs
            .get(self.active_tab)
            .and_then(|tab| tab.layout_root.as_ref());
        self.find.recompute(root);
    }

    fn step_find(&mut self, forward: bool) {
        self.url_focused = false;
        self.store.dismiss_autocomplete();
        if !self.find.open {
            self.find.show();
        }
        self.ensure_find_matches();
        if self.find.step(forward) {
            self.scroll_find_active_into_view();
        }
        self.mark_full_repaint();
    }

    fn scroll_find_active_into_view(&mut self) {
        let Some(rect) = self.find.active_rect() else {
            return;
        };
        let top = self.tabs[self.active_tab].scroll_y;
        let bottom = top + self.page_viewport_height();
        if rect.y < top {
            self.scroll(rect.y - top);
        } else if rect.y + rect.height > bottom {
            self.scroll(rect.y + rect.height - bottom);
        }
    }

    fn cycle_autocomplete(&mut self, forward: bool) {
        let typed = self.url_input.clone();
        if let Some(url) = self.store.cycle_autocomplete(&typed, forward) {
            self.url_input = url;
            self.mark_chrome_repaint();
        }
    }

    fn accept_autocomplete(&mut self) {
        let typed = self.url_input.clone();
        if let Some(url) = self.store.accept_autocomplete(&typed) {
            self.url_input = url;
            self.mark_chrome_repaint();
        }
    }

    /// Inline autocomplete suffix for the current address-bar text.
    pub fn autocomplete_suffix(&self) -> String {
        self.store.completion_suffix(&self.url_input)
    }

    fn inspector_overlay(&self) -> Option<BoxModelOverlay> {
        if !self.inspector_enabled {
            return None;
        }
        let node_id = self.hovered_node?;
        let tab = self.tabs.get(self.active_tab)?;
        let root = tab.layout_root.as_ref()?;
        let layout = droplet_layout::find_layout_box_by_node(root, node_id)?;
        let tag = tab
            .doc
            .as_ref()
            .and_then(|doc| doc.get(node_id))
            .map(|node| match &node.kind {
                NodeKind::Element(data) => data.tag_name.as_str(),
                NodeKind::Text(_) => "text",
                NodeKind::Document => "document",
            })
            .unwrap_or("box");
        let content = layout.dimensions.content;
        Some(BoxModelOverlay::from_dimensions(
            &layout.dimensions,
            format!(
                "{} #{} {:.0}\u{00d7}{:.0}",
                tag, node_id, content.width, content.height
            ),
        ))
    }

    pub fn render_into(&mut self, canvas: &mut Canvas) {
        self.start_deferred();
        self.ensure_find_matches();

        let find_highlights: Vec<FindHighlight> = if self.find.open {
            self.find
                .matches
                .iter()
                .enumerate()
                .map(|(idx, item)| FindHighlight {
                    rect: item.rect,
                    active: idx == self.find.active,
                })
                .collect()
        } else {
            Vec::new()
        };
        let inspector = self.inspector_overlay();
        let find_bar = if self.find.open {
            Some(FindBarView {
                query: &self.find.query,
                current: self.find.current(),
                total: self.find.total(),
                focused: !self.url_focused,
            })
        } else {
            None
        };
        let overlay = OverlayState {
            find_bar,
            find_highlights: &find_highlights,
            inspector: inspector.as_ref(),
        };
        let autocomplete_suffix = self.autocomplete_suffix();

        let staged = self.staged_render_damage.take();
        let pending = self.pending_damage.take();
        let damage = staged
            .or(pending)
            .map(|d| d.clamp(canvas.width, canvas.height));
        let is_sub_rect = damage.is_some_and(|d| {
            !d.is_empty()
                && (d.x > 0 || d.y > 0 || d.width < canvas.width || d.height < canvas.height)
        });
        let platform_clears_slice = matches!(self.present_backend, "x11-shm" | "wayland-shm");
        let can_partial = is_sub_rect
            && !platform_clears_slice
            && self
                .tabs
                .get(self.active_tab)
                .and_then(|t| t.display_list.as_ref())
                .is_some();

        let tab_titles: Vec<&str> = self.tabs.iter().map(|t| t.title.as_str()).collect();
        let tab = &self.tabs[self.active_tab];
        let display_list = tab.display_list.as_deref().unwrap_or(&[]);
        let chrome = ChromeState {
            url_input: &self.url_input,
            url_focused: self.url_focused,
            autocomplete_suffix: &autocomplete_suffix,
            can_back: !tab.back_stack.is_empty(),
            can_forward: !tab.forward_stack.is_empty(),
            status_text: &self.status_text,
            timing_badge: &tab.timing_badge,
            tab_titles: &tab_titles,
            active_tab: self.active_tab,
            scroll_y: tab.scroll_y,
            content_height: tab.content_height,
        };
        if can_partial {
            if let Some(dmg) = damage {
                render_frame_damaged(
                    canvas,
                    &mut self.font_engine,
                    display_list,
                    &chrome,
                    Some(&overlay),
                    dmg,
                );
                return;
            }
        }
        render_frame_ext(
            canvas,
            &mut self.font_engine,
            display_list,
            &chrome,
            Some(&overlay),
        );
    }

    pub fn render(&mut self) -> Canvas {
        let mut canvas = Canvas::new(
            self.viewport_width,
            self.viewport_height,
            Color::rgb(255, 255, 255),
        );
        self.pending_damage = Some(DamageRect::full(self.viewport_width, self.viewport_height));
        self.staged_render_damage = None;
        self.render_into(&mut canvas);
        canvas
    }
}

impl PlatformApp for Browser {
    fn set_waker(&mut self, waker: droplet_platform::Waker) {
        Browser::set_waker(self, waker);
    }

    fn poll_background(&mut self) -> bool {
        let navigated = self.poll_navigations();
        self.poll_webviews() || navigated
    }

    fn set_embed_parent(&mut self, handle: u64) {
        self.embed_parent = Some(handle);
    }

    fn embedded_view(&self) -> Option<droplet_platform::EmbeddedView> {
        Browser::embedded_view(self)
    }

    fn viewport_size(&self) -> (usize, usize) {
        (self.viewport_width, self.viewport_height)
    }

    fn url_focused(&self) -> bool {
        self.url_focused
    }

    fn needs_repaint(&self) -> bool {
        self.needs_repaint
    }

    fn set_needs_repaint(&mut self, dirty: bool) {
        self.needs_repaint = dirty;
        if !dirty {
            self.pending_damage = None;
            self.staged_render_damage = None;
        } else if self.pending_damage.is_none() {
            self.mark_full_repaint();
        }
    }

    fn take_damage(&mut self) -> Option<DamageRect> {
        if !self.needs_repaint {
            self.pending_damage = None;
            self.staged_render_damage = None;
            return None;
        }
        let w = self.viewport_width;
        let h = self.viewport_height;
        let dmg = self
            .pending_damage
            .take()
            .unwrap_or_else(|| DamageRect::full(w, h))
            .clamp(w, h);
        if dmg.is_empty() {
            self.staged_render_damage = None;
            None
        } else {
            self.staged_render_damage = Some(dmg);
            Some(dmg)
        }
    }

    fn set_present_backend(&mut self, backend: &'static str) {
        self.present_backend = backend;
    }

    fn active_title(&self) -> &str {
        Browser::active_title(self)
    }

    fn resize(&mut self, width: usize, height: usize) {
        Browser::resize(self, width, height);
    }

    fn scroll(&mut self, delta_y: f32) {
        Browser::scroll(self, delta_y);
    }

    fn handle_mouse_move(&mut self, x: f32, y: f32) {
        Browser::handle_mouse_move(self, x, y);
    }

    fn handle_mouse_down(&mut self, x: f32, y: f32) {
        Browser::handle_mouse_down(self, x, y);
    }

    fn handle_mouse_up(&mut self, x: f32, y: f32) {
        Browser::handle_mouse_up(self, x, y);
    }

    fn handle_text_input(&mut self, ch: char) {
        Browser::handle_text_input(self, ch);
    }

    fn handle_backspace(&mut self) {
        Browser::handle_backspace(self);
    }

    fn handle_submit_url(&mut self) {
        Browser::handle_submit_url(self);
    }

    fn wants_text_input(&self) -> bool {
        self.url_focused
            || self.find.open
            || self.focused_control().is_some_and(|node| {
                self.tabs[self.active_tab]
                    .doc
                    .as_ref()
                    .is_some_and(|doc| forms::is_text_like(doc, node))
            })
    }

    fn handle_enter(&mut self, shift: bool) {
        if self.url_focused {
            self.handle_submit_url();
        } else if self.find.open {
            self.step_find(!shift);
        } else {
            self.handle_activate();
        }
    }

    fn handle_key_navigation(&mut self, key: NavKey) {
        Browser::handle_key_navigation(self, key);
    }

    fn render_into(&mut self, canvas: &mut Canvas) {
        Browser::render_into(self, canvas);
    }

    fn handle_tab_focus(&mut self, backwards: bool) {
        Browser::handle_tab_focus(self, backwards);
    }

    fn handle_caret(&mut self, movement: CaretMove, extend: bool) {
        Browser::handle_caret(self, movement, extend);
    }

    fn handle_delete(&mut self) {
        Browser::handle_delete(self);
    }

    fn handle_activate(&mut self) {
        Browser::handle_activate(self);
    }

    fn handle_select_all(&mut self) {
        Browser::handle_select_all(self);
    }

    fn handle_copy(&mut self) {
        Browser::handle_copy(self);
    }

    fn handle_cut(&mut self) {
        Browser::handle_cut(self);
    }

    fn handle_paste(&mut self) {
        Browser::handle_paste(self);
    }
}

fn relayout_tab_state(tab: &mut TabState, ctx: &StyleContext, page_vh: f32) {
    let (Some(doc), Some(stylesheet)) = (tab.doc.as_ref(), tab.stylesheet.as_ref()) else {
        return;
    };
    let visual = tab.forms.visual(doc);
    let layout_root = compute_layout_with_ctx(doc, stylesheet, ctx, &tab.images);
    let display_list = build_display_list_with_forms(&layout_root, &visual);
    let content_height = compute_content_height(&layout_root);
    let max_scroll = (content_height - page_vh).max(0.0);
    tab.scroll_y = tab.scroll_y.clamp(0.0, max_scroll);
    tab.content_height = content_height;
    tab.layout_root = Some(layout_root);
    tab.display_list = Some(display_list);
    tab.dirty.style_and_layout = false;
    tab.dirty.paint = false;
    tab.layout_rebuilds += 1;
    tab.display_list_rebuilds += 1;
}

/// Status line shown while a tab's navigation is in flight.
fn loading_status(tab: &TabState) -> String {
    match &tab.pending {
        Some(p) if !p.url.host.is_empty() => format!("Loading {}…", p.url.host),
        Some(p) => format!("Loading {}…", p.url.as_str()),
        None => String::new(),
    }
}

pub(crate) fn compute_layout_with_ctx(
    doc: &Document,
    stylesheet: &Stylesheet,
    ctx: &StyleContext,
    images: &HashMap<String, Arc<DecodedImage>>,
) -> LayoutBox {
    if let Some(styled) = droplet_style::style_tree_with_context(doc, stylesheet, ctx) {
        droplet_layout::layout_tree(doc, &styled, ctx.viewport_width, ctx.viewport_height, images)
    } else {
        let fallback_doc = html::parse("<!DOCTYPE html><html><body></body></html>");
        let fallback_css = css::parse("");
        let fallback_styled =
            droplet_style::style_tree_with_context(&fallback_doc, &fallback_css, ctx)
                .expect("fallback html produces root");
        let empty_images = HashMap::new();
        droplet_layout::layout_tree(
            &fallback_doc,
            &fallback_styled,
            ctx.viewport_width,
            ctx.viewport_height,
            &empty_images,
        )
    }
}

pub(crate) fn compute_content_height(root: &LayoutBox) -> f32 {
    let mut max_bottom = 0.0f32;
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        let mb = node.dimensions.margin_box();
        let bottom = mb.y + mb.height;
        if bottom.is_finite() && bottom > max_bottom {
            max_bottom = bottom;
        }
        for child in &node.children {
            stack.push(child);
        }
    }
    max_bottom
}

#[cfg(test)]
mod tests {
    use super::*;
    use droplet_gfx::Rect;
    use droplet_layout::BoxType;
    use droplet_paint::DisplayCommand;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEST_DATA_SEQ: AtomicUsize = AtomicUsize::new(0);

    fn temp_data_dir(label: &str) -> std::path::PathBuf {
        let seq = TEST_DATA_SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "droplet_shell_{label}_{}_{seq}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// Every shell test gets its own store so parallel runs cannot see each
    /// other's history and the user's real data directory is never touched.
    fn test_browser(initial_url: &str, width: usize, height: usize) -> Browser {
        Browser::with_data_dir(
            initial_url,
            width,
            height,
            temp_data_dir("nav"),
        )
    }

    fn find_link_center(
        root: &LayoutBox,
        node: &LayoutBox,
        skip_href: Option<&str>,
    ) -> Option<(f32, f32, String)> {
        if let Some(href) = &node.href {
            if skip_href != Some(href.as_str()) {
                let rect = node.dimensions.border_box();
                if rect.width > 0.0 && rect.height > 0.0 {
                    let cx = rect.x + rect.width * 0.5;
                    let cy = rect.y + rect.height * 0.5;
                    if droplet_layout::hit_test_href(root, cx, cy) == Some(href.as_str()) {
                        return Some((cx, cy, href.clone()));
                    }
                }
            }
        }
        for child in &node.children {
            if let Some(found) = find_link_center(root, child, skip_href) {
                return Some(found);
            }
        }
        None
    }

    fn find_image_rect(node: &LayoutBox) -> Option<Rect> {
        if matches!(
            node.box_type,
            BoxType::Image {
                image: Some(_),
                ..
            }
        ) {
            return Some(node.dimensions.content);
        }
        for child in &node.children {
            if let Some(r) = find_image_rect(child) {
                return Some(r);
            }
        }
        None
    }

    fn png_crc32(parts: &[&[u8]]) -> u32 {
        let mut crc: u32 = 0xFFFF_FFFF;
        for part in parts {
            for &b in *part {
                crc ^= b as u32;
                for _ in 0..8 {
                    let mask = (crc & 1).wrapping_neg();
                    crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
                }
            }
        }
        !crc
    }

    fn png_adler32(data: &[u8]) -> u32 {
        let mut s1: u32 = 1;
        let mut s2: u32 = 0;
        for &b in data {
            s1 = (s1 + b as u32) % 65521;
            s2 = (s2 + s1) % 65521;
        }
        (s2 << 16) | s1
    }

    fn push_png_chunk(out: &mut Vec<u8>, tag: &[u8; 4], payload: &[u8]) {
        out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        out.extend_from_slice(tag);
        out.extend_from_slice(payload);
        out.extend_from_slice(&png_crc32(&[tag, payload]).to_be_bytes());
    }

    fn make_solid_png_2x2(r: u8, g: u8, b: u8, a: u8) -> Vec<u8> {
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut ihdr = Vec::with_capacity(13);
        ihdr.extend_from_slice(&2u32.to_be_bytes());
        ihdr.extend_from_slice(&2u32.to_be_bytes());
        ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8-bit RGBA
        push_png_chunk(&mut out, b"IHDR", &ihdr);

        let raw = [
            0, r, g, b, a, r, g, b, a, // row 0
            0, r, g, b, a, r, g, b, a, // row 1
        ];
        let mut zlib = vec![0x78, 0x01, 0x01];
        let len = raw.len() as u16;
        zlib.extend_from_slice(&len.to_le_bytes());
        zlib.extend_from_slice(&(!len).to_le_bytes());
        zlib.extend_from_slice(&raw);
        zlib.extend_from_slice(&png_adler32(&raw).to_be_bytes());
        push_png_chunk(&mut out, b"IDAT", &zlib);
        push_png_chunk(&mut out, b"IEND", &[]);
        out
    }

    fn encode_base64(input: &[u8]) -> String {
        const TBL: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
        for chunk in input.chunks(3) {
            let b0 = chunk[0] as usize;
            let b1 = *chunk.get(1).unwrap_or(&0) as usize;
            let b2 = *chunk.get(2).unwrap_or(&0) as usize;
            out.push(TBL[b0 >> 2] as char);
            out.push(TBL[((b0 & 0x03) << 4) | (b1 >> 4)] as char);
            if chunk.len() > 1 {
                out.push(TBL[((b1 & 0x0F) << 2) | (b2 >> 6)] as char);
            } else {
                out.push('=');
            }
            if chunk.len() > 2 {
                out.push(TBL[b2 & 0x3F] as char);
            } else {
                out.push('=');
            }
        }
        out
    }

    #[test]
    fn browser_loads_about_droplet_and_clicks_link() {
        assert_eq!(CHROME_HEIGHT, 74);
        let mut browser = test_browser("about:droplet", 1100, 760);
        assert_eq!(browser.active_url_str(), "about:droplet");
        assert!(
            !browser.tabs[0]
                .display_list
                .as_ref()
                .expect("display_list")
                .is_empty()
        );
        assert!(browser.status_text.is_empty());
        assert!(browser.tabs[0].timing_badge.ends_with(" ms"));

        let layout_root = browser.tabs[0].layout_root.as_ref().expect("layout_root");
        let (dx, dy, target_href) =
            find_link_center(layout_root, layout_root, Some("about:droplet"))
                .expect("about:droplet should have a clickable link to another page");
        let click_y = dy - browser.tabs[0].scroll_y + CHROME_HEIGHT as f32;
        if click_y >= CHROME_HEIGHT as f32
            && click_y < (browser.viewport_height - STATUS_BAR_HEIGHT) as f32
        {
            browser.handle_click(dx, click_y);
            assert_eq!(browser.active_url_str(), target_href);
        } else {
            browser.scroll(dy);
            let adjusted_y = dy - browser.tabs[0].scroll_y + CHROME_HEIGHT as f32;
            browser.handle_click(dx, adjusted_y);
            assert_eq!(browser.active_url_str(), target_href);
        }
        assert_eq!(browser.tabs[0].back_stack.len(), 1);
    }

    #[test]
    fn history_back_and_forward() {
        let mut browser = test_browser("about:droplet", 1024, 768);
        browser.navigate("about:roadmap");
        assert_eq!(browser.active_url_str(), "about:roadmap");
        assert_eq!(browser.tabs[0].back_stack.len(), 1);

        browser.navigate("about:test");
        assert_eq!(browser.active_url_str(), "about:test");
        assert_eq!(browser.tabs[0].back_stack.len(), 2);

        browser.go_back();
        assert_eq!(browser.active_url_str(), "about:roadmap");
        assert_eq!(browser.tabs[0].forward_stack.len(), 1);

        browser.go_back();
        assert_eq!(browser.active_url_str(), "about:droplet");
        assert_eq!(browser.tabs[0].forward_stack.len(), 2);

        browser.go_forward();
        assert_eq!(browser.active_url_str(), "about:roadmap");

        // Navigating to a new page clears forward stack
        browser.navigate("about:architecture");
        assert!(browser.tabs[0].forward_stack.is_empty());
    }

    #[test]
    fn address_bar_typing_backspace_and_submit() {
        let mut browser = test_browser("about:droplet", 1024, 768);
        let bar = chrome_hitboxes(1024, browser.tabs.len()).address_bar;
        browser.handle_click(bar.x + 10.0, bar.y + 10.0);
        assert!(browser.url_focused);

        while !browser.url_input.is_empty() {
            browser.handle_backspace();
        }
        for ch in "about:test".chars() {
            browser.handle_text_input(ch);
        }
        assert_eq!(browser.url_input, "about:test");

        browser.handle_submit_url();
        assert!(!browser.url_focused);
        assert_eq!(browser.active_url_str(), "about:test");
    }

    #[test]
    fn scroll_clamping_and_resize_relayout() {
        let mut browser = test_browser("about:roadmap", 900, 400);
        browser.scroll(-100.0);
        assert_eq!(browser.tabs[0].scroll_y, 0.0);

        browser.scroll(100_000.0);
        let expected_max =
            (browser.tabs[0].content_height - browser.page_viewport_height()).max(0.0);
        assert!((browser.tabs[0].scroll_y - expected_max).abs() < 0.01);

        // Resizing larger clamps scroll_y cleanly and marks repaint
        browser.needs_repaint = false;
        browser.resize(1200, 900);
        assert!(browser.needs_repaint);
        let new_max = (browser.tabs[0].content_height - browser.page_viewport_height()).max(0.0);
        assert!(browser.tabs[0].scroll_y <= new_max + 0.01);
    }

    #[test]
    fn hover_status_pill_empty_by_default_and_clears_on_unhover() {
        let mut browser = test_browser("about:test", 1024, 768);
        assert!(browser.status_text.is_empty());

        let layout_root = browser.tabs[0].layout_root.as_ref().expect("layout_root");
        let (dx, dy, href) =
            find_link_center(layout_root, layout_root, None).expect("about:test has links");
        let view_y = dy + CHROME_HEIGHT as f32;
        browser.needs_repaint = false;

        browser.handle_mouse_move(dx, view_y);
        assert_eq!(browser.status_text, href);
        assert!(browser.needs_repaint);

        browser.needs_repaint = false;
        browser.handle_mouse_move(dx, view_y);
        assert!(!browser.needs_repaint);

        // Moving into chrome (unhover) clears status_text
        browser.handle_mouse_move(5.0, 5.0);
        assert!(browser.status_text.is_empty());
        assert!(browser.needs_repaint);
    }

    #[test]
    fn invalid_and_missing_urls_render_friendly_error_without_panic() {
        let mut browser = test_browser("about:droplet", 1024, 768);
        browser.navigate("ftp://example.com");
        assert_eq!(browser.active_title(), "Navigation Error");
        assert!(!browser.tabs[0].display_list.as_ref().unwrap().is_empty());

        browser.navigate("file:///tmp/droplet_definitely_missing_987654321.html");
        assert_eq!(browser.active_title(), "Load Error");
        let canvas = browser.render();
        assert_eq!(canvas.width, 1024);
        assert_eq!(canvas.height, 768);
    }

    #[test]
    fn relative_file_link_navigation() {
        let dir = std::env::temp_dir().join(format!("droplet_nav_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let page1 = dir.join("one.html");
        let page2 = dir.join("two.html");
        std::fs::write(
            &page1,
            "<html><head><title>One</title></head><body><a href=\"two.html\">Go Two</a></body></html>",
        )
        .unwrap();
        std::fs::write(
            &page2,
            "<html><head><title>Two</title></head><body><p>Reached Two</p></body></html>",
        )
        .unwrap();

        let url1 = format!("file://{}", page1.display());
        let mut browser = test_browser(&url1, 800, 600);
        assert_eq!(browser.active_title(), "One");

        let layout_root = browser.tabs[0].layout_root.as_ref().unwrap();
        let (dx, dy, href) = find_link_center(layout_root, layout_root, None).unwrap();
        assert_eq!(href, "two.html");
        browser.handle_click(dx, dy + CHROME_HEIGHT as f32);
        assert_eq!(browser.active_title(), "Two");
        assert!(browser.active_url_str().ends_with("/two.html"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn multi_tab_lifecycle_and_auto_hibernation() {
        let mut browser = test_browser("about:droplet", 1024, 768);
        assert_eq!(browser.tabs.len(), 1);
        assert_eq!(browser.active_tab, 0);
        assert!(!browser.tabs[0].hibernated);

        browser.new_tab("about:roadmap");
        browser.new_tab("about:architecture");
        assert_eq!(browser.tabs.len(), 3);
        assert_eq!(browser.active_tab, 2);
        assert_eq!(browser.tabs.iter().filter(|t| !t.hibernated).count(), 3);

        // Opening 4th tab past MAX_WARM_TABS=3 auto-hibernates oldest tab (tab 0)
        browser.new_tab("about:test");
        assert_eq!(browser.tabs.len(), 4);
        assert_eq!(browser.active_tab, 3);
        assert_eq!(
            browser.tabs.iter().filter(|t| !t.hibernated).count(),
            MAX_WARM_TABS
        );
        assert!(browser.tabs[0].hibernated);
        assert!(browser.tabs[0].doc.is_none());
        assert!(browser.tabs[0].stylesheet.is_none());
        assert!(browser.tabs[0].images.is_empty());
        assert!(browser.tabs[0].layout_root.is_none());
        assert!(browser.tabs[0].display_list.is_none());

        // Scroll tab 3 and remember scroll_y across hibernation/wakeup
        browser.scroll(120.0);
        let saved_scroll = browser.tabs[3].scroll_y;
        assert!(saved_scroll > 0.0);

        // Switch back to hibernated tab 0: transparently reloads & hibernates oldest warm tab (tab 1)
        browser.switch_tab(0);
        assert_eq!(browser.active_tab, 0);
        assert_eq!(browser.active_url_str(), "about:droplet");
        assert!(!browser.tabs[0].hibernated);
        assert!(browser.tabs[0].doc.is_some());
        assert!(browser.tabs[0].layout_root.is_some());
        assert!(browser.tabs[1].hibernated);
        assert_eq!(
            browser.tabs.iter().filter(|t| !t.hibernated).count(),
            MAX_WARM_TABS
        );

        // Explicitly hibernate tab 3 and wake it up, verifying scroll_y restoration
        browser.hibernate_tab(3);
        assert!(browser.tabs[3].hibernated);
        assert!(browser.tabs[3].layout_root.is_none());
        browser.switch_tab(3);
        assert!(!browser.tabs[3].hibernated);
        assert!(browser.tabs[3].layout_root.is_some());
        assert!((browser.tabs[3].scroll_y - saved_scroll).abs() < 0.01);

        // Keyboard tab navigation & close down to last tab resets to about:droplet
        browser.handle_key_navigation(NavKey::PrevTab);
        assert_eq!(browser.active_tab, 2);
        browser.handle_key_navigation(NavKey::NextTab);
        assert_eq!(browser.active_tab, 3);

        while browser.tabs.len() > 1 {
            browser.close_tab(browser.active_tab);
        }
        assert_eq!(browser.tabs.len(), 1);
        browser.navigate("about:test");
        assert_eq!(browser.active_url_str(), "about:test");

        // Closing the last remaining tab resets it to about:droplet
        browser.close_tab(0);
        assert_eq!(browser.tabs.len(), 1);
        assert_eq!(browser.active_tab, 0);
        assert_eq!(browser.active_url_str(), "about:droplet");
    }

    #[test]
    fn external_stylesheet_and_data_uri_image_load_and_render() {
        let png = make_solid_png_2x2(255, 0, 0, 255);
        let data_uri = format!("data:image/png;base64,{}", encode_base64(&png));
        let html = format!(
            "<!DOCTYPE html><html><head><title>Ext CSS + Img</title>\
             <style>html, body {{ margin: 0; padding: 0; }} .box {{ background-color: #111111; }}</style>\
             <link rel=\"stylesheet\" href=\"/theme.css\">\
             </head><body><div class=\"box\">\
             <img src=\"{data_uri}\" width=\"24\" height=\"24\" alt=\"red\">\
             <img src=\"{data_uri}\" width=\"12\" height=\"12\" alt=\"dup\">\
             <img src=\"/missing.png\" alt=\"missing\">\
             </div></body></html>"
        );
        let css =
            "body { margin: 0; padding: 0; } .box { background-color: #10b981; padding: 12px; }";

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let port = listener.local_addr().unwrap().port();

        let server = std::thread::spawn(move || {
            for _ in 0..3 {
                let Ok((mut stream, _)) = listener.accept() else {
                    break;
                };
                let mut req_bytes = Vec::new();
                let mut buf = [0u8; 1024];
                loop {
                    let n = stream.read(&mut buf).unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    req_bytes.extend_from_slice(&buf[..n]);
                    if req_bytes.windows(4).any(|w| w == b"\r\n\r\n")
                        || req_bytes.len() >= 16384
                    {
                        break;
                    }
                }
                let req = String::from_utf8_lossy(&req_bytes);
                let path = req
                    .lines()
                    .next()
                    .and_then(|l| l.split_whitespace().nth(1))
                    .unwrap_or("/");

                let (status_line, ctype, body): (&str, &str, &[u8]) = match path {
                    "/index.html" => (
                        "HTTP/1.1 200 OK",
                        "text/html; charset=utf-8",
                        html.as_bytes(),
                    ),
                    "/theme.css" => (
                        "HTTP/1.1 200 OK",
                        "text/css; charset=utf-8",
                        css.as_bytes(),
                    ),
                    _ => ("HTTP/1.1 404 Not Found", "text/plain", b"not found"),
                };
                let header = format!(
                    "{status_line}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(body);
            }
        });

        let url = format!("http://127.0.0.1:{port}/index.html");
        let mut browser = test_browser(&url, 600, 400);
        let _ = server.join();

        assert_eq!(browser.active_title(), "Ext CSS + Img");
        assert_eq!(browser.tabs[0].images.len(), 1);
        assert!(browser.tabs[0].images.contains_key(&data_uri));

        let img_rect = find_image_rect(browser.tabs[0].layout_root.as_ref().unwrap())
            .expect("decoded image should appear in layout tree");
        assert!((img_rect.width - 24.0).abs() < 0.01);
        assert!((img_rect.height - 24.0).abs() < 0.01);

        let canvas = browser.render();
        // Top-left padding of .box should use #10b981 from external /theme.css
        let box_px = canvas.pixels[CHROME_HEIGHT * canvas.width + 2];
        assert_eq!(box_px, 0x0010_B981);

        // Center of decoded red PNG image should be pure red
        let ix = (img_rect.x + img_rect.width * 0.5) as usize;
        let iy = (img_rect.y + img_rect.height * 0.5) as usize + CHROME_HEIGHT;
        assert_eq!(canvas.pixels[iy * canvas.width + ix], 0x00FF_0000);
    }

    #[test]
    fn hover_and_active_restyle_only_when_uses_interaction_is_true() {
        let dir =
            std::env::temp_dir().join(format!("droplet_interact_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let static_path = dir.join("static.html");
        let interactive_path = dir.join("interactive.html");

        std::fs::write(
            &static_path,
            "<!DOCTYPE html><html><head><style>\
             html, body { margin: 0; padding: 0; }\
             .card { display: block; width: 120px; height: 40px; background-color: #cbd5e1; }\
             </style></head><body><div class=\"card\"></div></body></html>",
        )
        .unwrap();

        std::fs::write(
            &interactive_path,
            "<!DOCTYPE html><html><head><style>\
             html, body { margin: 0; padding: 0; }\
             .btn { display: block; width: 120px; height: 40px; background-color: #e2e8f0; }\
             .btn:hover { background-color: #2563eb; }\
             .btn:active { background-color: #1e3a8a; }\
             </style></head><body><div class=\"btn\"></div></body></html>",
        )
        .unwrap();

        // Part A: static page has uses_interaction == false and skips restyle/repaint
        let static_url = format!("file://{}", static_path.display());
        let mut browser = test_browser(&static_url, 600, 400);
        assert!(!browser.tabs[0].uses_interaction);
        browser.needs_repaint = false;

        let hit_y = CHROME_HEIGHT as f32 + 20.0;
        browser.handle_mouse_move(60.0, hit_y);
        assert!(browser.hovered_node.is_some());
        assert!(!browser.needs_repaint);

        browser.handle_mouse_down(60.0, hit_y);
        assert!(browser.active_node.is_some());
        assert!(!browser.needs_repaint);

        browser.handle_mouse_up(60.0, hit_y);
        assert!(browser.active_node.is_none());
        assert!(!browser.needs_repaint);

        // Part B: interactive page triggers restyle on state transition only
        let interactive_url = format!("file://{}", interactive_path.display());
        browser.navigate(&interactive_url);
        assert!(browser.tabs[0].uses_interaction);

        let has_rect_color = |b: &Browser, expected: Color| -> bool {
            b.tabs[0]
                .display_list
                .as_ref()
                .unwrap()
                .iter()
                .any(|cmd| matches!(cmd, DisplayCommand::SolidRect { color, .. } if *color == expected))
        };

        assert!(has_rect_color(&browser, Color::rgb(226, 232, 240)));

        browser.needs_repaint = false;
        browser.handle_mouse_move(60.0, hit_y);
        assert!(browser.needs_repaint);
        assert!(has_rect_color(&browser, Color::rgb(37, 99, 235)));

        // Moving inside the same hovered element does not trigger redundant restyle
        browser.needs_repaint = false;
        browser.handle_mouse_move(62.0, hit_y + 1.0);
        assert!(!browser.needs_repaint);

        // Mouse down transitions to :active
        browser.handle_mouse_down(62.0, hit_y + 1.0);
        assert!(browser.needs_repaint);
        assert!(has_rect_color(&browser, Color::rgb(30, 58, 138)));

        // Mouse up clears :active back to :hover
        browser.needs_repaint = false;
        browser.handle_mouse_up(62.0, hit_y + 1.0);
        assert!(browser.needs_repaint);
        assert!(has_rect_color(&browser, Color::rgb(37, 99, 235)));

        // Unhovering restores base style
        browser.needs_repaint = false;
        browser.handle_mouse_move(60.0, CHROME_HEIGHT as f32 + 120.0);
        assert!(browser.needs_repaint);
        assert!(has_rect_color(&browser, Color::rgb(226, 232, 240)));

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn js_browser(url: &str, width: usize, height: usize) -> Browser {
        Browser::with_history_store(
            url,
            width,
            height,
            HistoryStore::open(temp_data_dir("js")),
            BrowserOptions { js: true },
        )
    }

    #[test]
    fn javascript_is_disabled_by_default_and_scripts_are_not_run() {
        let dir = std::env::temp_dir().join(format!("droplet_js_off_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let page = dir.join("off.html");
        std::fs::write(
            &page,
            "<!DOCTYPE html><html><head><title>Тихо</title></head><body>\
             <script>console.log('не должно выполниться');</script><p>Привет</p></body></html>",
        )
        .unwrap();
        let url = format!("file://{}", page.display());
        let browser = Browser::with_history_store(
            &url,
            800,
            600,
            HistoryStore::open(temp_data_dir("js-off")),
            BrowserOptions::default(),
        );
        assert!(!browser.js_enabled());
        assert!(browser.active_js_logs().is_empty());
        assert!(browser.active_js_errors().is_empty());
        assert_eq!(browser.active_title(), "Тихо");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn headless_inline_and_multi_script_execution_order_and_cyrillic() {
        let dir = std::env::temp_dir().join(format!("droplet_js_inline_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let page = dir.join("scripts.html");
        std::fs::write(
            &page,
            "<!DOCTYPE html><html><head><title>JS Order</title>\
             <script>var order = ['first']; function greet(имя) { return 'Привет, ' + имя; }</script>\
             <script type=\"application/json\">{\"ignored\": true}</script>\
             </head><body>\
             <script>order.push('second'); console.log(greet('Мир'));</script>\
             <script>order.push('third'); console.log(order.join('>'));</script>\
             </body></html>",
        )
        .unwrap();

        let url = format!("file://{}", page.display());
        let browser = js_browser(&url, 800, 600);
        assert!(browser.active_js_errors().is_empty());
        assert_eq!(
            browser.active_js_logs(),
            &["Привет, Мир".to_string(), "first>second>third".to_string()]
        );
        assert_eq!(browser.status_text, "JS: first>second>third");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn headless_script_runtime_and_syntax_error_isolation() {
        let dir = std::env::temp_dir().join(format!("droplet_js_errors_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let page = dir.join("errors.html");
        std::fs::write(
            &page,
            "<!DOCTYPE html><html><head><title>JS Errors</title></head><body>\
             <script>let base = 40;</script>\
             <script>let broken = ;</script>\
             <script>console.log('before'); null.boom;</script>\
             <script>console.log('after', base + 2);</script>\
             </body></html>",
        )
        .unwrap();

        let url = format!("file://{}", page.display());
        let browser = js_browser(&url, 800, 600);
        assert_eq!(
            browser.active_js_logs(),
            &["before".to_string(), "after 42".to_string()]
        );
        assert_eq!(browser.active_js_errors().len(), 2);
        assert!(browser.active_js_errors()[0].contains("SyntaxError"));
        assert!(browser.active_js_errors()[1].contains("TypeError"));
        assert!(browser.status_text.starts_with("JS error: TypeError"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn history_and_bookmarks_persist_across_browsers() {
        let dir = temp_data_dir("persist");
        {
            let mut browser = Browser::with_data_dir("about:droplet", 1024, 768, &dir);
            browser.navigate("about:test");
            assert_eq!(browser.active_url_str(), "about:test");
            assert!(browser
                .store
                .history()
                .iter()
                .any(|entry| entry.url == "about:test"));

            browser.handle_key_navigation(NavKey::ToggleBookmark);
            assert!(browser.store.is_bookmarked("about:test"));
            assert_eq!(browser.status_text, "Bookmark added");
            assert!(browser.store.bookmarks().iter().any(|bookmark| {
                bookmark.url == "about:test" && bookmark.title.contains("Visual Test")
            }));
        }

        let reopened = Browser::with_data_dir("about:blank", 1024, 768, &dir);
        assert!(reopened.store.is_bookmarked("about:test"));
        assert!(reopened
            .store
            .history()
            .iter()
            .any(|entry| entry.url == "about:test"));
    }

    #[test]
    fn only_successful_navigations_are_recorded() {
        let dir = temp_data_dir("success");
        let mut browser = Browser::with_data_dir("about:blank", 1024, 768, &dir);
        // about:blank (also the placeholder behind a deferred GUI start) is never recorded.
        assert!(browser.store.history().is_empty());
        browser.navigate("about:droplet");
        assert_eq!(browser.store.history().len(), 1);

        browser.navigate("ftp://example.com");
        assert_eq!(browser.active_title(), "Navigation Error");
        assert_eq!(browser.store.history().len(), 1);

        browser.navigate("about:history?clear=1");
        assert!(browser.store.history().is_empty());

        browser.navigate("about:test");
        assert_eq!(browser.store.history().len(), 1);
        assert_eq!(browser.store.history()[0].url, "about:test");
    }

    #[test]
    fn about_history_and_bookmarks_render_and_remove_entries() {
        let dir = temp_data_dir("about-pages");
        let mut browser = Browser::with_data_dir("about:droplet", 1024, 768, &dir);
        browser.navigate("about:test");
        browser.handle_key_navigation(NavKey::ToggleBookmark);

        browser.navigate("about:history");
        assert_eq!(browser.active_url_str(), "about:history");
        assert!(browser.active_title().contains("History"));
        let dump = browser.active_dom_dump();
        assert!(dump.contains("History"));
        assert!(dump.contains("about:test"));
        assert!(!browser.tabs[0].display_list.as_ref().unwrap().is_empty());
        let canvas = browser.render();
        assert_eq!((canvas.width, canvas.height), (1024, 768));

        let remove = format!(
            "about:history?remove={}",
            droplet_net::user_pages::percent_encode("about:test")
        );
        browser.navigate(&remove);
        assert_eq!(browser.active_url_str(), "about:history");
        assert!(!browser
            .store
            .history()
            .iter()
            .any(|entry| entry.url == "about:test"));

        browser.navigate("about:bookmarks");
        assert_eq!(browser.active_url_str(), "about:bookmarks");
        assert!(browser.active_title().contains("Bookmarks"));
        assert!(browser.active_dom_dump().contains("about:test"));

        let remove = format!(
            "about:bookmarks?remove={}",
            droplet_net::user_pages::percent_encode("about:test")
        );
        browser.navigate(&remove);
        assert!(!browser.store.is_bookmarked("about:test"));
        assert!(browser.active_dom_dump().contains("No bookmarks yet"));

        browser.navigate("about:history?clear=1");
        assert!(browser.active_dom_dump().contains("No history yet"));
        let _ = browser.render();
    }

    #[test]
    fn autocomplete_suffix_accept_cycle_and_escape() {
        let dir = temp_data_dir("autocomplete");
        let mut browser = Browser::with_data_dir("about:droplet", 1024, 768, &dir);
        browser.navigate("about:test");

        let bar = chrome_hitboxes(1024, browser.tabs.len()).address_bar;
        browser.handle_click(bar.x + 10.0, bar.y + 10.0);
        assert!(browser.url_focused);
        while !browser.url_input.is_empty() {
            browser.handle_backspace();
        }
        for ch in "about:t".chars() {
            browser.handle_text_input(ch);
        }
        assert_eq!(browser.autocomplete_suffix(), "est");

        browser.handle_key_navigation(NavKey::AcceptCompletion);
        assert_eq!(browser.url_input, "about:test");
        assert_eq!(browser.autocomplete_suffix(), "");

        // Clear and type a broader prefix with two history candidates.
        while !browser.url_input.is_empty() {
            browser.handle_backspace();
        }
        for ch in "about:".chars() {
            browser.handle_text_input(ch);
        }
        assert!(!browser.autocomplete_suffix().is_empty());

        browser.handle_key_navigation(NavKey::Down);
        assert_eq!(browser.url_input, "about:droplet");
        browser.handle_key_navigation(NavKey::Up);
        assert_eq!(browser.url_input, "about:test");

        browser.handle_key_navigation(NavKey::Escape);
        assert!(!browser.url_focused);
        assert_eq!(browser.url_input, "about:test");
        assert_eq!(browser.autocomplete_suffix(), "");
    }

    #[test]
    fn ctrl_digit_switching_ignores_out_of_range_tabs() {
        let mut browser = test_browser("about:droplet", 1024, 768);
        browser.new_tab("about:test");
        browser.new_tab("about:roadmap");
        assert_eq!(browser.active_tab, 2);

        browser.handle_key_navigation(NavKey::SwitchTab(0));
        assert_eq!(browser.active_tab, 0);
        browser.handle_key_navigation(NavKey::SwitchTab(8));
        assert_eq!(browser.active_tab, 0);
        browser.handle_key_navigation(NavKey::SwitchTab(1));
        assert_eq!(browser.active_tab, 1);

        browser.handle_key_navigation(NavKey::NextTab);
        assert_eq!(browser.active_tab, 2);
        browser.handle_key_navigation(NavKey::PrevTab);
        assert_eq!(browser.active_tab, 1);
    }

    #[test]
    fn find_cyrillic_wraps_and_escape_closes() {
        let dir = temp_data_dir("find");
        std::fs::create_dir_all(&dir).unwrap();
        let page = dir.join("find.html");
        std::fs::write(
            &page,
            "<!DOCTYPE html><html><head><style>html, body { margin: 0; padding: 0; }</style></head>\
             <body><p>Привет мир ПРИВЕТ и Ёж</p></body></html>",
        )
        .unwrap();
        let url = format!("file://{}", page.display());
        let mut browser = test_browser(&url, 1024, 768);

        browser.handle_key_navigation(NavKey::Find);
        assert!(browser.find.open);
        assert!(!browser.url_focused);
        for ch in "привет".chars() {
            browser.handle_text_input(ch);
        }
        browser.ensure_find_matches();
        assert_eq!(browser.find.total(), 2);
        assert_eq!(browser.find.current(), 1);

        browser.handle_key_navigation(NavKey::FindNext);
        assert_eq!(browser.find.current(), 2);
        browser.handle_key_navigation(NavKey::FindNext);
        assert_eq!(browser.find.current(), 1);
        browser.handle_key_navigation(NavKey::FindPrev);
        assert_eq!(browser.find.current(), 2);

        <Browser as PlatformApp>::handle_enter(&mut browser, true);
        assert_eq!(browser.find.current(), 1);
        <Browser as PlatformApp>::handle_enter(&mut browser, false);
        assert_eq!(browser.find.current(), 2);

        browser.handle_key_navigation(NavKey::Escape);
        assert!(!browser.find.open);
        assert!(browser.find.matches.is_empty());
        let _ = browser.render();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn headless_external_scripts_and_virtual_clock_timers() {
        let html = "<!DOCTYPE html><html><head><title>Ext JS</title>\
             <script>let trace = ['inline-1'];</script>\
             <script src=\"/lib.js\"></script>\
             <script src=\"/missing.js\"></script>\
             <script>\
               trace.push('inline-2');\
               let count = 0;\
               let iv = setInterval(function () {\
                 count += 1;\
                 console.log('tick-' + count);\
                 if (count >= 2) { clearInterval(iv); }\
               }, 10);\
               setTimeout(function () { console.log(trace.join('|')); }, 15);\
             </script>\
             </head><body><p>OK</p></body></html>";
        let lib_js = "trace.push('external'); console.log('lib loaded');";

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let port = listener.local_addr().unwrap().port();

        let server = std::thread::spawn(move || {
            for _ in 0..3 {
                let Ok((mut stream, _)) = listener.accept() else {
                    break;
                };
                let mut req_bytes = Vec::new();
                let mut buf = [0u8; 1024];
                loop {
                    let n = stream.read(&mut buf).unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    req_bytes.extend_from_slice(&buf[..n]);
                    if req_bytes.windows(4).any(|w| w == b"\r\n\r\n")
                        || req_bytes.len() >= 16384
                    {
                        break;
                    }
                }
                let req = String::from_utf8_lossy(&req_bytes);
                let path = req
                    .lines()
                    .next()
                    .and_then(|l| l.split_whitespace().nth(1))
                    .unwrap_or("/");

                let (status_line, ctype, body): (&str, &str, &[u8]) = match path {
                    "/index.html" => (
                        "HTTP/1.1 200 OK",
                        "text/html; charset=utf-8",
                        html.as_bytes(),
                    ),
                    "/lib.js" => (
                        "HTTP/1.1 200 OK",
                        "application/javascript; charset=utf-8",
                        lib_js.as_bytes(),
                    ),
                    _ => ("HTTP/1.1 404 Not Found", "text/plain", b"not found"),
                };
                let header = format!(
                    "{status_line}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(body);
            }
        });

        let url = format!("http://127.0.0.1:{port}/index.html");
        let mut browser = js_browser(&url, 600, 400);
        let _ = server.join();

        assert_eq!(browser.active_js_logs(), &["lib loaded".to_string()]);
        assert_eq!(browser.active_js_errors().len(), 1);
        assert!(browser.active_js_errors()[0].contains("HTTP status 404"));

        browser.tick_js(10.0);
        browser.tick_js(15.0);
        browser.advance_js(10.0);
        assert_eq!(
            browser.active_js_logs(),
            &[
                "lib loaded".to_string(),
                "tick-1".to_string(),
                "inline-1|external|inline-2".to_string(),
                "tick-2".to_string(),
            ]
        );
        assert_eq!(browser.tabs[0].js.pending_timer_count(), 0);
    }

    #[test]
    fn host_hook_and_tab_hibernation_reset_js_runtime() {
        let mut browser = js_browser("about:droplet", 800, 600);
        assert!(browser.tabs[0].js.runtime.is_none());
        assert!(browser.status_text.is_empty());

        browser.configure_active_js(|rt| {
            rt.define_global("hostVersion", droplet_js::vm::Value::Number(7.0));
        });
        assert!(browser.tabs[0].js.runtime.is_some());
        let val = browser.tabs[0]
            .js
            .runtime
            .as_ref()
            .unwrap()
            .get_global("hostVersion")
            .unwrap();
        assert_eq!(val.as_number(), Some(7.0));

        browser.hibernate_tab(0);
        assert!(browser.tabs[0].js.runtime.is_none());
    }

    #[test]
    fn inspector_overlay_tracks_hovered_box() {
        let mut browser = test_browser("about:test", 1024, 768);
        browser.handle_key_navigation(NavKey::ToggleInspector);
        assert!(browser.inspector_enabled);

        let hit_y = CHROME_HEIGHT as f32 + 30.0;
        browser.needs_repaint = false;
        browser.handle_mouse_move(30.0, hit_y);
        let node_id = browser.hovered_node.expect("hovered layout node");
        assert!(browser.needs_repaint);

        let overlay = browser.inspector_overlay().expect("inspector overlay");
        assert!(overlay.label.contains(&format!("#{node_id}")));
        assert!(overlay.label.contains('\u{00d7}'));
        assert!(overlay.margin.width >= overlay.border.width);
        assert!(overlay.border.width >= overlay.padding.width);
        assert!(overlay.padding.width >= overlay.content.width);

        browser.handle_key_navigation(NavKey::ToggleInspector);
        assert!(!browser.inspector_enabled);
        assert!(browser.inspector_overlay().is_none());

        browser.handle_key_navigation(NavKey::ToggleInspector);
        browser.hibernate_tab(0);
        assert!(browser.inspector_overlay().is_none());
    }

    #[test]
    fn headless_e2e_form_focus_editing_per_tab_and_tcp_get_and_post_submit() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let base_url = format!("http://127.0.0.1:{port}/form");

        let server = std::thread::spawn(move || {
            for _ in 0..4 {
                let (mut sock, _) = listener.accept().unwrap();
                let mut raw = Vec::new();
                let mut buf = [0u8; 2048];
                let header_end = loop {
                    let n = sock.read(&mut buf).unwrap();
                    if n == 0 {
                        break raw.len();
                    }
                    raw.extend_from_slice(&buf[..n]);
                    if let Some(pos) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                        break pos + 4;
                    }
                };
                let head = String::from_utf8_lossy(&raw[..header_end]).to_string();
                let first_line = head.lines().next().unwrap_or("");
                if first_line.starts_with("GET /form ") {
                    let html = "<!DOCTYPE html><html><head><title>Form</title>\
                        <style>input:focus { border-color: #2563eb; }</style></head><body>\
                        <form action=\"/search\" method=\"get\">\
                        <input type=\"text\" name=\"q\" placeholder=\"Поиск\">\
                        <input type=\"checkbox\" name=\"agree\" value=\"yes\">\
                        <select name=\"lang\"><option value=\"ru\">RU</option><option value=\"en\">EN</option></select>\
                        <input type=\"submit\" value=\"Go\">\
                        </form>\
                        <form action=\"/login\" method=\"post\">\
                        <input type=\"password\" name=\"pw\">\
                        <button type=\"submit\" name=\"act\" value=\"send\">Login</button>\
                        </form>\
                        <p>Selectable text</p>\
                        </body></html>";
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        html.len(),
                        html
                    );
                    sock.write_all(resp.as_bytes()).unwrap();
                } else if first_line.starts_with("GET /search?") {
                    let body = format!("<!DOCTYPE html><html><body>GET:{first_line}</body></html>");
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    sock.write_all(resp.as_bytes()).unwrap();
                } else if first_line.starts_with("POST /login ") {
                    let cl: usize = head
                        .lines()
                        .find_map(|l| {
                            let (k, v) = l.split_once(':')?;
                            k.trim()
                                .eq_ignore_ascii_case("content-length")
                                .then(|| v.trim().parse().ok())
                                .flatten()
                        })
                        .unwrap_or(0);
                    while raw.len() < header_end + cl {
                        let n = sock.read(&mut buf).unwrap();
                        if n == 0 {
                            break;
                        }
                        raw.extend_from_slice(&buf[..n]);
                    }
                    let req_body = String::from_utf8_lossy(&raw[header_end..header_end + cl]).to_string();
                    let body = format!("<!DOCTYPE html><html><body>POST:{req_body}</body></html>");
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    sock.write_all(resp.as_bytes()).unwrap();
                }
            }
        });

        let mut browser = Browser::new(&base_url, 800, 600);
        // Tab 0 focuses first input via Tab, types Cyrillic, toggles checkbox, cycles select
        browser.handle_tab_focus(false);
        assert!(browser.tabs[0].forms.focused.is_some());
        for ch in "Мир".chars() {
            browser.handle_text_input(ch);
        }
        let doc = browser.tabs[0].doc.as_ref().unwrap();
        let q_node = browser.tabs[0].forms.focused.unwrap();
        assert_eq!(forms::value_of(doc, q_node), "Мир");

        // Tab to checkbox and toggle with Space
        browser.handle_tab_focus(false);
        browser.handle_text_input(' ');
        // Tab to select and cycle with Right arrow
        browser.handle_tab_focus(false);
        browser.handle_caret(CaretMove::Right, false);

        // Open Tab 1 and verify focus state is per-tab
        browser.new_tab("about:droplet");
        assert!(browser.tabs[1].forms.focused.is_none());
        browser.switch_tab(0);
        assert!(browser.tabs[0].forms.focused.is_some());

        // Select all on page after Esc blurs control
        browser.handle_key_navigation(NavKey::Escape);
        assert!(browser.tabs[0].forms.focused.is_none());
        browser.handle_select_all();
        assert!(browser.tabs[0].forms.page_selection.is_some());

        // Focus q_node again and submit GET form via Enter
        browser.handle_tab_focus(false);
        browser.handle_activate();
        assert!(browser.active_url_str().contains("/search?q=%D0%9C%D0%B8%D1%80&agree=yes&lang=en"));

        // Go back to /form, navigate to POST form, type password, and submit via button click
        browser.go_back();
        browser.handle_tab_focus(false); // q
        browser.handle_tab_focus(false); // checkbox
        browser.handle_tab_focus(false); // select
        browser.handle_tab_focus(false); // submit
        browser.handle_tab_focus(false); // password
        for ch in "abc".chars() {
            browser.handle_text_input(ch);
        }
        browser.handle_tab_focus(false); // Login button
        browser.handle_activate();
        let doc_after_post = browser.tabs[0].doc.as_ref().unwrap();
        let rendered_text = doc_after_post.text_content(doc_after_post.root());
        assert!(rendered_text.contains("POST:pw=abc&act=send"));

        server.join().unwrap();
    }

    #[test]
    fn mouse_drag_selection_and_shift_arrow_extend_in_control_and_page() {
        let dir = std::env::temp_dir().join(format!("droplet_drag_sel_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let page_path = dir.join("drag.html");
        std::fs::write(
            &page_path,
            "<!DOCTYPE html><html><body style=\"margin: 0; padding: 10px;\">\
             <p id=\"p1\">Hello world</p>\
             <input type=\"text\" value=\"Привет\">\
             </body></html>",
        )
        .unwrap();
        let url = format!("file://{}", page_path.display());
        let mut browser = Browser::new(&url, 600, 400);

        // Drag over rendered paragraph text
        let root = browser.tabs[0].layout_root.as_ref().unwrap();
        let entries = forms::text_entries(root);
        assert!(!entries.is_empty());
        let mut stack = vec![root];
        let mut p_rect = None;
        while let Some(b) = stack.pop() {
            if matches!(b.box_type, BoxType::Text { .. }) {
                p_rect = Some(b.dimensions.content);
                break;
            }
            for c in b.children.iter().rev() {
                stack.push(c);
            }
        }
        let p_rect = p_rect.expect("text rect");
        let y = p_rect.y + p_rect.height * 0.5 + CHROME_HEIGHT as f32;
        browser.handle_mouse_down(p_rect.x + 1.0, y);
        browser.handle_mouse_move(p_rect.x + p_rect.width - 1.0, y);
        browser.handle_mouse_up(p_rect.x + p_rect.width - 1.0, y);
        let page_sel = browser.tabs[0]
            .forms
            .page_selection
            .expect("page selection after drag");
        let selected = forms::page_selection_text(
            browser.tabs[0].layout_root.as_ref().unwrap(),
            &page_sel,
        );
        assert!(!selected.is_empty());

        // Focus text input, drag inside it, and verify Shift+Left extends/shrinks selection
        let input_node = forms::focusable_controls(browser.tabs[0].doc.as_ref().unwrap())[0];
        let input_rect = forms::find_control_box(
            browser.tabs[0].layout_root.as_ref().unwrap(),
            input_node,
        )
        .unwrap()
        .dimensions
        .content;
        let iy = input_rect.y + input_rect.height * 0.5 + CHROME_HEIGHT as f32;
        browser.handle_mouse_down(input_rect.x + 1.0, iy);
        browser.handle_mouse_move(input_rect.x + input_rect.width - 1.0, iy);
        browser.handle_mouse_up(input_rect.x + input_rect.width - 1.0, iy);
        assert_eq!(browser.tabs[0].forms.focused, Some(input_node));
        let sel_range = browser.tabs[0]
            .forms
            .control_selection()
            .expect("control drag selection");
        let doc = browser.tabs[0].doc.as_ref().unwrap();
        assert_eq!(
            forms::selected_slice(&forms::value_of(doc, input_node), sel_range.0, sel_range.1),
            Some("Привет")
        );

        // Shift+Left shrinks selection by 1 char (2 bytes for Cyrillic)
        browser.handle_caret(CaretMove::Left, true);
        let sel_range2 = browser.tabs[0].forms.control_selection().unwrap();
        assert_eq!(
            forms::selected_slice(
                &forms::value_of(browser.tabs[0].doc.as_ref().unwrap(), input_node),
                sel_range2.0,
                sel_range2.1
            ),
            Some("Приве")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dirty_flags_skip_redundant_relayout_when_clean_or_paint_only() {
        let mut browser = test_browser("about:test", 640, 480);
        assert_eq!(browser.tabs[0].layout_rebuilds, 1);
        assert_eq!(browser.tabs[0].display_list_rebuilds, 1);
        assert!(browser.tabs[0].dirty.is_clean());

        // Clean flush performs zero work.
        browser.needs_repaint = false;
        browser.flush_active_dirty();
        assert!(!browser.needs_repaint);
        assert_eq!(browser.tabs[0].layout_rebuilds, 1);
        assert_eq!(browser.tabs[0].display_list_rebuilds, 1);

        // Paint-only dirty rebuilds display list without relayout.
        browser.tabs[0].dirty.mark_paint();
        browser.flush_active_dirty();
        assert!(browser.needs_repaint);
        assert_eq!(browser.tabs[0].layout_rebuilds, 1);
        assert_eq!(browser.tabs[0].display_list_rebuilds, 2);

        // Style/layout + title dirty updates title and runs one layout + paint pass.
        browser.tabs[0]
            .doc
            .as_mut()
            .unwrap()
            .set_document_title("Updated Title");
        browser.tabs[0].dirty.mark_title();
        browser.tabs[0].dirty.mark_style_and_layout();
        browser.flush_active_dirty();
        assert_eq!(browser.active_title(), "Updated Title");
        assert_eq!(browser.tabs[0].layout_rebuilds, 2);
        assert_eq!(browser.tabs[0].display_list_rebuilds, 3);
        assert!(browser.tabs[0].dirty.is_clean());
    }

    #[test]
    fn reactive_dom_scripts_events_bubbling_prevent_default_and_timers() {
        let dir = std::env::temp_dir().join(format!("droplet_drop008_e2e_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let page_path = dir.join("index.html");
        let target_path = dir.join("other.html");
        std::fs::write(&target_path, "<!DOCTYPE html><html><title>Other</title><body>Other</body></html>").unwrap();
        std::fs::write(
            &page_path,
            "<!DOCTYPE html><html><head><title>Initial</title></head>\
             <body style=\"margin: 0; padding: 10px;\">\
               <div id=\"parent\">\
                 <button id=\"stop_btn\" type=\"button\">Stop</button>\
                 <a id=\"guarded_link\" href=\"other.html\">Blocked Link</a>\
               </div>\
               <form id=\"f\" action=\"other.html\" method=\"get\">\
                 <input id=\"txt\" name=\"q\" type=\"text\" value=\"\">\
                 <input id=\"chk\" name=\"agree\" type=\"checkbox\">\
                 <button id=\"sub\" type=\"submit\">Submit</button>\
               </form>\
               <p id=\"status\">init</p>\
               <script>\
                 document.title = 'Reactive Page';\
                 var p = document.getElementById('status');\
                 p.textContent = 'Привет';\
                 p.style.color = 'rgb(220, 38, 38)';\
                 p.classList.add('ready');\
                 var item = document.createElement('span');\
                 item.textContent = '!';\
                 p.appendChild(item);\
                 var logCount = 0;\
                 document.getElementById('parent').addEventListener('click', function () {\
                   console.log('parent-clicked');\
                 });\
                 document.getElementById('stop_btn').addEventListener('click', function (e) {\
                   e.stopPropagation();\
                   console.log('btn-stopped:' + e.currentTarget.id);\
                 });\
                 document.getElementById('guarded_link').addEventListener('click', function (e) {\
                   e.preventDefault();\
                   console.log('link-prevented:' + e.defaultPrevented);\
                 });\
                 var txt = document.getElementById('txt');\
                 txt.addEventListener('keydown', function (e) {\
                   console.log('kd:' + e.key);\
                   if (e.key === 'x') { e.preventDefault(); }\
                 });\
                 txt.addEventListener('input', function (e) {\
                   console.log('in:' + e.value);\
                 });\
                 txt.addEventListener('keyup', function (e) {\
                   console.log('ku:' + e.key);\
                 });\
                 document.getElementById('chk').addEventListener('change', function (e) {\
                   console.log('chg:' + e.checked);\
                 });\
                 document.getElementById('f').addEventListener('submit', function (e) {\
                   e.preventDefault();\
                   console.log('submit-prevented');\
                 });\
                 setTimeout(function () {\
                   document.getElementById('status').textContent = 'timer-fired';\
                 }, 50);\
               </script>\
             </body></html>",
        )
        .unwrap();

        let url = format!("file://{}", page_path.display());
        let mut browser = js_browser(&url, 800, 600);
        assert_eq!(browser.active_title(), "Reactive Page");
        let doc = browser.tabs[0].doc.as_ref().unwrap();
        let status_id = doc.get_element_by_id("status").unwrap();
        assert_eq!(doc.text_content_of(status_id), "Привет!");
        assert!(doc.class_list_contains(status_id, "ready"));
        assert_eq!(doc.style_prop_get(status_id, "color"), "rgb(220, 38, 38)");

        // Click stop_btn -> stopPropagation prevents parent click log
        let stop_btn = doc.get_element_by_id("stop_btn").unwrap();
        let stop_rect = forms::find_control_box(
            browser.tabs[0].layout_root.as_ref().unwrap(),
            stop_btn,
        )
        .unwrap()
        .dimensions
        .padding_box();
        browser.handle_click(
            stop_rect.x + stop_rect.width * 0.5,
            stop_rect.y + stop_rect.height * 0.5 + CHROME_HEIGHT as f32,
        );
        assert_eq!(browser.active_js_logs(), &["btn-stopped:stop_btn"]);

        // Click guarded_link -> bubbles to parent AND preventDefault cancels navigation
        let root_box = browser.tabs[0].layout_root.as_ref().unwrap();
        let (lx, ly, href) = find_link_center(root_box, root_box, None).expect("link rect");
        assert_eq!(href, "other.html");
        browser.handle_click(lx, ly + CHROME_HEIGHT as f32);
        assert_eq!(browser.active_url_str(), url);
        assert!(browser.active_js_logs().contains(&"link-prevented:true".to_string()));
        assert!(browser.active_js_logs().contains(&"parent-clicked".to_string()));

        // Focus text input via JS, test keydown preventDefault + input + keyup
        browser.eval_active_js("document.getElementById('txt').focus();").unwrap();
        let txt_id = browser.tabs[0].doc.as_ref().unwrap().get_element_by_id("txt").unwrap();
        assert_eq!(browser.tabs[0].forms.focused, Some(txt_id));
        browser.handle_text_input('x'); // prevented!
        browser.handle_text_input('a');
        assert_eq!(
            forms::value_of(browser.tabs[0].doc.as_ref().unwrap(), txt_id),
            "a"
        );
        assert!(browser.active_js_logs().contains(&"kd:x".to_string()));
        assert!(browser.active_js_logs().contains(&"kd:a".to_string()));
        assert!(browser.active_js_logs().contains(&"in:a".to_string()));
        assert!(browser.active_js_logs().contains(&"ku:a".to_string()));

        // Checkbox change event
        let chk_id = browser.tabs[0].doc.as_ref().unwrap().get_element_by_id("chk").unwrap();
        let chk_rect = forms::find_control_box(
            browser.tabs[0].layout_root.as_ref().unwrap(),
            chk_id,
        )
        .unwrap()
        .dimensions
        .border_box();
        browser.handle_click(
            chk_rect.x + chk_rect.width * 0.5,
            chk_rect.y + chk_rect.height * 0.5 + CHROME_HEIGHT as f32,
        );
        assert!(browser.active_js_logs().contains(&"chg:true".to_string()));

        // Form submit prevented by JS submit listener
        let sub_id = browser.tabs[0].doc.as_ref().unwrap().get_element_by_id("sub").unwrap();
        let sub_rect = forms::find_control_box(
            browser.tabs[0].layout_root.as_ref().unwrap(),
            sub_id,
        )
        .unwrap()
        .dimensions
        .border_box();
        browser.handle_click(
            sub_rect.x + sub_rect.width * 0.5,
            sub_rect.y + sub_rect.height * 0.5 + CHROME_HEIGHT as f32,
        );
        assert_eq!(browser.active_url_str(), url);
        assert!(browser.active_js_logs().contains(&"submit-prevented".to_string()));

        // Idle tick before timer fires runs ZERO layout/display list rebuilds!
        let before_layouts = browser.tabs[0].layout_rebuilds;
        let before_dl = browser.tabs[0].display_list_rebuilds;
        browser.needs_repaint = false;
        browser.tick(10.0);
        assert!(!browser.needs_repaint);
        assert_eq!(browser.tabs[0].layout_rebuilds, before_layouts);
        assert_eq!(browser.tabs[0].display_list_rebuilds, before_dl);

        // Timer fires at 50ms -> mutates DOM -> triggers exactly 1 relayout
        browser.tick(50.0);
        assert!(browser.needs_repaint);
        assert_eq!(browser.tabs[0].layout_rebuilds, before_layouts + 1);
        assert_eq!(
            browser.tabs[0].doc.as_ref().unwrap().text_content_of(status_id),
            "timer-fired"
        );
        assert!(browser.active_js_errors().is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn js_tracker_script_blocking_and_bounded_console_logs() {
        let dir = std::env::temp_dir().join(format!("droplet_drop008_sec_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let page_path = dir.join("sec.html");
        std::fs::write(
            &page_path,
            "<!DOCTYPE html><html><body>\
             <script src=\"https://www.google-analytics.com/analytics.js\"></script>\
             <script>\
               for (var i = 0; i < 300; i = i + 1) { console.log('line-' + i); }\
             </script>\
             </body></html>",
        )
        .unwrap();
        let url = format!("file://{}", page_path.display());
        let browser = js_browser(&url, 640, 480);

        // Tracker script blocked cleanly via TrackerFilter without crashing
        assert!(browser
            .active_js_errors()
            .iter()
            .any(|e| e.contains("google-analytics")));
        // Console log buffer capped at MAX_JS_LOG_ENTRIES (256)
        assert_eq!(browser.active_js_logs().len(), crate::js::MAX_JS_LOG_ENTRIES);
        assert_eq!(browser.active_js_logs().first().map(String::as_str), Some("line-44"));
        assert_eq!(browser.active_js_logs().last().map(String::as_str), Some("line-299"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn browser_damage_tracking_and_present_backend_roundtrip() {
        let dir = std::env::temp_dir().join(format!("droplet_drop010_dmg_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let page_path = dir.join("tall.html");
        std::fs::write(
            &page_path,
            "<!DOCTYPE html><html><body style=\"margin:0\"><div style=\"height:1200px;background:#223344\">tall</div></body></html>",
        )
        .unwrap();
        let url = format!("file://{}", page_path.display());
        let mut browser = test_browser(&url, 640, 480);

        assert_eq!(browser.present_backend(), "software");
        <Browser as PlatformApp>::set_present_backend(&mut browser, "wayland-shm");
        assert_eq!(browser.present_backend(), "wayland-shm");

        assert_eq!(
            <Browser as PlatformApp>::take_damage(&mut browser),
            Some(DamageRect::full(640, 480))
        );
        <Browser as PlatformApp>::set_needs_repaint(&mut browser, false);
        assert_eq!(<Browser as PlatformApp>::take_damage(&mut browser), None);

        // Chrome interaction -> tight chrome damage rect only.
        browser.handle_key_navigation(NavKey::FocusUrl);
        browser.handle_text_input('x');
        assert_eq!(
            <Browser as PlatformApp>::take_damage(&mut browser),
            Some(chrome_damage_rect(640, 480))
        );
        <Browser as PlatformApp>::set_needs_repaint(&mut browser, false);

        // Scroll -> tight viewport damage rect only.
        browser.scroll(100.0);
        assert_eq!(
            <Browser as PlatformApp>::take_damage(&mut browser),
            Some(viewport_damage_rect(640, 480))
        );
        <Browser as PlatformApp>::set_needs_repaint(&mut browser, false);

        // Resize -> full damage rect for new size.
        browser.resize(800, 600);
        assert_eq!(
            <Browser as PlatformApp>::take_damage(&mut browser),
            Some(DamageRect::full(800, 600))
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn browser_render_into_partial_damage_matches_full_render() {
        let dir =
            std::env::temp_dir().join(format!("droplet_drop010_partial_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let page_path = dir.join("partial.html");
        std::fs::write(
            &page_path,
            "<!DOCTYPE html><html><head><title>Partial</title></head>\
             <body style=\"margin:0;background:#eef2f7\">\
             <div style=\"height:200px;background:#1d4ed8;color:#ffffff\">Header</div>\
             <div style=\"height:900px;background:#0f172a;color:#f8fafc\">Body</div>\
             </body></html>",
        )
        .unwrap();
        let url = format!("file://{}", page_path.display());
        let mut browser = test_browser(&url, 640, 480);

        // Initial full paint into retained canvas.
        let mut retained = Canvas::new(640, 480, Color::rgb(255, 255, 255));
        assert_eq!(
            <Browser as PlatformApp>::take_damage(&mut browser),
            Some(DamageRect::full(640, 480))
        );
        browser.render_into(&mut retained);
        <Browser as PlatformApp>::set_needs_repaint(&mut browser, false);

        // Chrome-only edit: partial render_frame_damaged matches full render() pixel-for-pixel.
        browser.handle_key_navigation(NavKey::FocusUrl);
        browser.handle_text_input('?');
        assert_eq!(
            <Browser as PlatformApp>::take_damage(&mut browser),
            Some(chrome_damage_rect(640, 480))
        );
        browser.render_into(&mut retained);
        <Browser as PlatformApp>::set_needs_repaint(&mut browser, false);
        let expected_after_chrome = browser.render();
        assert_eq!(retained.pixels, expected_after_chrome.pixels);

        // Viewport-only scroll: partial render_frame_damaged matches full render() pixel-for-pixel.
        browser.handle_key_navigation(NavKey::Escape);
        browser.render_into(&mut retained);
        <Browser as PlatformApp>::set_needs_repaint(&mut browser, false);
        browser.scroll(150.0);
        assert_eq!(
            <Browser as PlatformApp>::take_damage(&mut browser),
            Some(viewport_damage_rect(640, 480))
        );
        browser.render_into(&mut retained);
        <Browser as PlatformApp>::set_needs_repaint(&mut browser, false);
        let expected_after_scroll = browser.render();
        assert_eq!(retained.pixels, expected_after_scroll.pixels);

        // Backends that pre-clear their slice to white before render_into always get a full paint.
        <Browser as PlatformApp>::set_present_backend(&mut browser, "wayland-shm");
        browser.scroll(40.0);
        let _ = <Browser as PlatformApp>::take_damage(&mut browser);
        let mut cleared_shm = Canvas::new(640, 480, Color::rgb(255, 255, 255));
        browser.render_into(&mut cleared_shm);
        let expected_after_shm = browser.render();
        assert_eq!(cleared_shm.pixels, expected_after_shm.pixels);

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- DROP-110: background navigation ----------------------------------------------

    /// Local HTTP server: `/slow/<title>` answers after `delay`, any other path immediately.
    /// Serves `conns` connections, then exits.
    fn slow_server(conns: usize, delay: std::time::Duration) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            for _ in 0..conns {
                let Ok((mut sock, _)) = listener.accept() else { return };
                let mut buf = [0u8; 2048];
                let n = sock.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
                if path.starts_with("/slow/") {
                    std::thread::sleep(delay);
                }
                let title = match path.rsplit('/').next() {
                    Some("ru") => "Привет".to_string(),
                    Some(other) => other.to_string(),
                    None => "root".to_string(),
                };
                let body = format!("<html><head><title>{title}</title></head><body><p>{title}</p></body></html>");
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = sock.write_all(resp.as_bytes());
            }
        });
        (base, handle)
    }

    fn install_test_waker(browser: &mut Browser) -> std::sync::mpsc::Receiver<()> {
        let (tx, rx) = std::sync::mpsc::channel();
        browser.set_waker(Arc::new(move || {
            let _ = tx.send(());
        }));
        rx
    }

    const WAKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

    #[test]
    fn background_navigation_keeps_ui_responsive_then_commits() {
        let (base, server) = slow_server(1, std::time::Duration::from_millis(500));
        let mut browser = test_browser("about:test", 800, 600);
        let old_title = browser.active_title().to_string();
        let wake = install_test_waker(&mut browser);

        let start = Instant::now();
        browser.navigate(&format!("{base}/slow/ru"));
        assert!(start.elapsed() < std::time::Duration::from_millis(250), "navigate() blocked");
        assert!(browser.is_loading());
        assert!(browser.status_text.starts_with("Loading 127.0.0.1"), "{}", browser.status_text);
        assert!(browser.url_input.ends_with("/slow/ru"));
        // The old page stays live: scrolling and rendering do not wait for the network.
        assert_eq!(browser.active_title(), old_title);
        browser.scroll(40.0);
        let _ = browser.render();
        assert!(start.elapsed() < std::time::Duration::from_millis(400));
        assert!(browser.tabs[0].back_stack.is_empty(), "history must change only on commit");

        wake.recv_timeout(WAKE_TIMEOUT).expect("waker fired");
        assert!(browser.poll_navigations());
        assert!(!browser.is_loading());
        assert_eq!(browser.active_title(), "Привет");
        assert_eq!(browser.tabs[0].back_stack.len(), 1);
        assert!(browser.tabs[0].current_url.as_str().ends_with("/slow/ru"));
        assert_eq!(browser.tabs[0].scroll_y, 0.0);
        server.join().unwrap();
    }

    #[test]
    fn superseded_navigation_result_is_dropped() {
        let (base, server) = slow_server(2, std::time::Duration::from_millis(400));
        let mut browser = test_browser("about:test", 800, 600);
        let wake = install_test_waker(&mut browser);

        browser.navigate(&format!("{base}/slow/first"));
        browser.navigate(&format!("{base}/fast/second"));
        wake.recv_timeout(WAKE_TIMEOUT).expect("first wake");
        wake.recv_timeout(WAKE_TIMEOUT).expect("second wake");
        browser.poll_navigations();

        assert_eq!(browser.active_title(), "second");
        assert!(!browser.is_loading());
        // Only the committed navigation touched history.
        assert_eq!(browser.tabs[0].back_stack.len(), 1);
        assert_eq!(browser.tabs[0].back_stack[0].as_str(), "about:test");
        server.join().unwrap();
    }

    #[test]
    fn escape_stops_loading_and_keeps_page_and_history() {
        let (base, server) = slow_server(1, std::time::Duration::from_millis(300));
        let mut browser = test_browser("about:test", 800, 600);
        let old_title = browser.active_title().to_string();
        let wake = install_test_waker(&mut browser);

        browser.navigate(&format!("{base}/slow/stopped"));
        assert!(browser.is_loading());
        browser.handle_key_navigation(NavKey::Escape);
        assert!(!browser.is_loading());
        assert_eq!(browser.url_input, "about:test");
        assert!(!browser.status_text.starts_with("Loading"));

        wake.recv_timeout(WAKE_TIMEOUT).expect("worker still finishes");
        assert!(!browser.poll_navigations(), "stopped result must be discarded");
        assert_eq!(browser.active_title(), old_title);
        assert!(browser.tabs[0].back_stack.is_empty());
        server.join().unwrap();
    }

    #[test]
    fn closing_a_loading_tab_discards_its_result() {
        let (base, server) = slow_server(1, std::time::Duration::from_millis(300));
        let mut browser = test_browser("about:test", 800, 600);
        browser.new_tab("about:droplet");
        let wake = install_test_waker(&mut browser);

        browser.navigate(&format!("{base}/slow/closed"));
        assert!(browser.is_loading());
        browser.close_tab(1);
        assert_eq!(browser.tabs.len(), 1);

        wake.recv_timeout(WAKE_TIMEOUT).expect("worker finishes");
        assert!(!browser.poll_navigations());
        assert_eq!(browser.tabs[0].current_url.as_str(), "about:test");
        server.join().unwrap();
    }

    #[test]
    fn background_back_and_forward_update_stacks_on_commit() {
        let mut browser = test_browser("about:test", 800, 600);
        browser.navigate("about:droplet");
        assert_eq!(browser.tabs[0].back_stack.len(), 1);
        let wake = install_test_waker(&mut browser);

        browser.go_back();
        assert!(browser.is_loading());
        assert_eq!(browser.tabs[0].back_stack.len(), 1, "unchanged until commit");
        wake.recv_timeout(WAKE_TIMEOUT).expect("back wake");
        assert!(browser.poll_navigations());
        assert_eq!(browser.tabs[0].current_url.as_str(), "about:test");
        assert!(browser.tabs[0].back_stack.is_empty());
        assert_eq!(browser.tabs[0].forward_stack.len(), 1);

        browser.go_forward();
        wake.recv_timeout(WAKE_TIMEOUT).expect("forward wake");
        assert!(browser.poll_navigations());
        assert_eq!(browser.tabs[0].current_url.as_str(), "about:droplet");
        assert_eq!(browser.tabs[0].back_stack.len(), 1);
        assert!(browser.tabs[0].forward_stack.is_empty());
    }

    #[test]
    fn background_load_into_inactive_tab_commits_without_stealing_focus() {
        let (base, server) = slow_server(1, std::time::Duration::from_millis(200));
        let mut browser = test_browser("about:test", 800, 600);
        let wake = install_test_waker(&mut browser);
        browser.navigate(&format!("{base}/slow/background"));
        // Open a second tab (about: page, also async) and stay on it.
        browser.new_tab("about:blank");
        wake.recv_timeout(WAKE_TIMEOUT).expect("wake 1");
        wake.recv_timeout(WAKE_TIMEOUT).expect("wake 2");
        browser.poll_navigations();
        assert_eq!(browser.active_tab, 1);
        assert_eq!(browser.tabs[0].title, "background");
        assert!(browser.tabs.iter().all(|t| t.pending.is_none()));
        assert_eq!(browser.url_input, "about:blank");
        server.join().unwrap();
    }

    #[test]
    fn deferred_start_loads_in_background_once_waker_is_installed() {
        let (base, server) = slow_server(1, std::time::Duration::from_millis(200));
        let url = format!("{base}/slow/ru");
        let start = Instant::now();
        let mut browser = Browser::new_deferred(&url, 800, 600);
        assert!(start.elapsed() < std::time::Duration::from_millis(150), "constructor blocked");
        assert_eq!(browser.url_input, url);
        assert_eq!(browser.tabs[0].current_url.as_str(), "about:blank");

        let wake = install_test_waker(&mut browser);
        assert!(browser.is_loading());
        wake.recv_timeout(WAKE_TIMEOUT).expect("wake");
        assert!(browser.poll_navigations());
        assert_eq!(browser.active_title(), "Привет");
        assert!(browser.tabs[0].back_stack.is_empty(), "initial load is not a history entry");
        server.join().unwrap();
    }

    #[test]
    fn deferred_start_without_waker_loads_before_first_frame() {
        let mut browser = Browser::new_deferred("about:test", 800, 600);
        assert_eq!(browser.tabs[0].current_url.as_str(), "about:blank");
        let _ = browser.render();
        assert_eq!(browser.tabs[0].current_url.as_str(), "about:test");
        assert!(!browser.is_loading());
    }
}
