pub mod loader;
pub mod tab;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use droplet_dom::{html, Document, NodeId};
use droplet_font::FontEngine;
use droplet_gfx::{Canvas, Color};
use droplet_image::DecodedImage;
use droplet_layout::LayoutBox;
use droplet_paint::{
    build_display_list, chrome_hitboxes, render_frame, ChromeState, CHROME_HEIGHT,
    STATUS_BAR_HEIGHT,
};
use droplet_platform::PlatformApp;
use droplet_style::{css, StyleContext, Stylesheet};
use droplet_url::Url;

pub use droplet_platform::NavKey;
pub use tab::{TabState, MAX_WARM_TABS};

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
    Viewport { href: Option<String> },
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
    base_status_text: String,
    pub needs_repaint: bool,
    font_engine: FontEngine,
    hovered_node: Option<NodeId>,
    active_node: Option<NodeId>,
    pressed_target: Option<PressedTarget>,
    active_seq_counter: u64,
}

impl Browser {
    pub fn new(initial_url: &str, width: usize, height: usize) -> Self {
        let initial_tab = TabState::create_tab("Droplet", 1);

        let mut browser = Self {
            tabs: vec![initial_tab],
            active_tab: 0,
            viewport_width: width.max(1),
            viewport_height: height.max(1),
            url_input: initial_url.to_string(),
            url_focused: false,
            status_text: String::new(),
            base_status_text: String::new(),
            needs_repaint: true,
            font_engine: FontEngine::new(),
            hovered_node: None,
            active_node: None,
            pressed_target: None,
            active_seq_counter: 1,
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

    pub fn navigate(&mut self, input: &str) {
        self.load_input_into_tab(self.active_tab, input, true);
    }

    fn load_input_into_tab(&mut self, tab_idx: usize, input: &str, push_history: bool) {
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
                if push_history {
                    let prev_url = self.tabs[tab_idx].current_url.clone();
                    self.tabs[tab_idx].back_stack.push(prev_url);
                    self.tabs[tab_idx].forward_stack.clear();
                }
                self.load_url_into_tab(tab_idx, target_url);
            }
            Err(err) => {
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

    fn load_url_into_tab(&mut self, tab_idx: usize, url: Url) {
        self.tabs[tab_idx].current_url = url.clone();
        if tab_idx == self.active_tab {
            self.url_input = url.as_str().to_string();
            self.url_focused = false;
        }

        let load_start = Instant::now();
        let page = loader::load_url_document(&url);
        self.tabs[tab_idx].current_url = page.final_url.clone();
        if tab_idx == self.active_tab {
            self.url_input = page.final_url.as_str().to_string();
        }
        self.apply_loaded_page(tab_idx, page, load_start);
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
        }

        let ctx = self.style_context_for_tab(tab_idx);
        let layout_root =
            compute_layout_with_ctx(&page.doc, &page.stylesheet, &ctx, &page.images);
        let display_list = build_display_list(&layout_root);
        let content_height = compute_content_height(&layout_root);
        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;

        let title = page
            .title_override
            .or_else(|| page.doc.find_title())
            .unwrap_or_else(|| self.tabs[tab_idx].current_url.as_str().to_string());

        let tab = &mut self.tabs[tab_idx];
        tab.title = title;
        tab.doc = Some(page.doc);
        tab.stylesheet = Some(page.stylesheet);
        tab.uses_interaction = page.uses_interaction;
        tab.images = page.images;
        tab.layout_root = Some(layout_root);
        tab.display_list = Some(display_list);
        tab.scroll_y = 0.0;
        tab.content_height = content_height;
        tab.timing_badge = format!("{elapsed_ms:.1} ms");
        tab.hibernated = false;

        if tab_idx == self.active_tab {
            self.base_status_text = page.status_override.clone();
            self.status_text = page.status_override;
            self.needs_repaint = true;
        }
    }

    pub fn go_back(&mut self) {
        let idx = self.active_tab;
        if let Some(prev) = self.tabs[idx].back_stack.pop() {
            let curr = self.tabs[idx].current_url.clone();
            self.tabs[idx].forward_stack.push(curr);
            self.load_url_into_tab(idx, prev);
        }
    }

    pub fn go_forward(&mut self) {
        let idx = self.active_tab;
        if let Some(next) = self.tabs[idx].forward_stack.pop() {
            let curr = self.tabs[idx].current_url.clone();
            self.tabs[idx].back_stack.push(curr);
            self.load_url_into_tab(idx, next);
        }
    }

    pub fn reload(&mut self) {
        let idx = self.active_tab;
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
            self.needs_repaint = true;
        } else if index == self.active_tab {
            self.active_tab = index.min(self.tabs.len() - 1);
            self.activate_current_tab();
        } else {
            self.needs_repaint = true;
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
        self.active_seq_counter += 1;
        self.tabs[idx].last_active_seq = self.active_seq_counter;
        self.base_status_text.clear();
        self.status_text.clear();

        if self.tabs[idx].hibernated {
            let saved_scroll = self.tabs[idx].scroll_y;
            let url = self.tabs[idx].current_url.clone();
            self.load_url_into_tab(idx, url);
            let max_s = self.max_scroll();
            self.tabs[idx].scroll_y = saved_scroll.clamp(0.0, max_s);
        } else {
            self.url_input = self.tabs[idx].current_url.as_str().to_string();
        }

        self.enforce_warm_tab_limit();
        self.needs_repaint = true;
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
            self.needs_repaint = true;
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

        self.needs_repaint = true;
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
            self.needs_repaint = true;
        }
    }

    fn style_context_for_tab(&self, tab_idx: usize) -> StyleContext {
        let (hovered_node, active_node) = if tab_idx == self.active_tab {
            (self.hovered_node, self.active_node)
        } else {
            (None, None)
        };
        StyleContext {
            viewport_width: self.viewport_width as f32,
            viewport_height: self.page_viewport_height().max(1.0),
            hovered_node,
            active_node,
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
        if (self.hovered_node, self.active_node) != prev_state
            && self.tabs[self.active_tab].uses_interaction
        {
            self.restyle_active_tab();
            self.needs_repaint = true;
        }
    }

    fn update_hover_status(&mut self, hit_href: Option<&str>) {
        let new_status = match hit_href {
            Some(href) => href.to_string(),
            None => self.base_status_text.clone(),
        };
        if new_status != self.status_text {
            self.status_text = new_status;
            self.needs_repaint = true;
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
                    self.needs_repaint = true;
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
                    self.needs_repaint = true;
                }
                self.go_back();
            }
            ChromeTarget::ForwardBtn => {
                if self.url_focused {
                    self.url_focused = false;
                    self.needs_repaint = true;
                }
                self.go_forward();
            }
            ChromeTarget::ReloadBtn => {
                if self.url_focused {
                    self.url_focused = false;
                    self.needs_repaint = true;
                }
                self.reload();
            }
            ChromeTarget::HomeBtn => {
                if self.url_focused {
                    self.url_focused = false;
                    self.needs_repaint = true;
                }
                self.navigate(HOME_URL);
            }
            ChromeTarget::AddressBar => {
                if !self.url_focused {
                    self.url_focused = true;
                    self.needs_repaint = true;
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

    pub fn handle_mouse_move(&mut self, x: f32, y: f32) {
        let (new_hovered, hit_href) = self.hit_test_viewport(x, y);
        self.update_interaction_state(new_hovered, self.active_node);
        self.update_hover_status(hit_href.as_deref());
    }

    pub fn handle_mouse_down(&mut self, x: f32, y: f32) {
        if let Some(target) = self.chrome_target_at(x, y) {
            self.pressed_target = Some(PressedTarget::Chrome(target));
            let should_focus = target == ChromeTarget::AddressBar;
            if self.url_focused != should_focus {
                self.url_focused = should_focus;
                self.needs_repaint = true;
            }
            self.update_interaction_state(None, None);
            self.update_hover_status(None);
            return;
        }

        if self.url_focused {
            self.url_focused = false;
            self.needs_repaint = true;
        }

        let vp_top = CHROME_HEIGHT as f32;
        let vp_bottom = self.viewport_height.saturating_sub(STATUS_BAR_HEIGHT) as f32;
        if y < vp_top || y >= vp_bottom {
            self.pressed_target = None;
            self.update_interaction_state(None, None);
            return;
        }

        let (hit_node, hit_href) = self.hit_test_viewport(x, y);
        self.pressed_target = Some(PressedTarget::Viewport { href: hit_href });
        self.update_interaction_state(hit_node, hit_node);
    }

    pub fn handle_mouse_up(&mut self, x: f32, y: f32) {
        let pressed = self.pressed_target.take();
        let (new_hovered, hit_href) = self.hit_test_viewport(x, y);

        self.update_interaction_state(new_hovered, None);
        self.update_hover_status(hit_href.as_deref());

        match pressed {
            Some(PressedTarget::Chrome(target)) => {
                if self.chrome_target_at(x, y) == Some(target) {
                    self.activate_chrome_target(target);
                }
            }
            Some(PressedTarget::Viewport {
                href: Some(down_href),
            }) if hit_href.as_deref() == Some(down_href.as_str()) => {
                self.navigate(&down_href);
            }
            _ => {}
        }
    }

    pub fn handle_click(&mut self, x: f32, y: f32) {
        self.pressed_target = None;
        if let Some(target) = self.chrome_target_at(x, y) {
            self.activate_chrome_target(target);
            return;
        }

        if self.url_focused {
            self.url_focused = false;
            self.needs_repaint = true;
        }

        let hovered = self.hovered_node;
        self.update_interaction_state(hovered, None);

        let (_, hit_href) = self.hit_test_viewport(x, y);
        if let Some(href) = hit_href {
            self.navigate(&href);
        }
    }

    pub fn handle_text_input(&mut self, ch: char) {
        if !self.url_focused || ch.is_control() {
            return;
        }
        self.url_input.push(ch);
        self.needs_repaint = true;
    }

    pub fn handle_backspace(&mut self) {
        if !self.url_focused {
            return;
        }
        if self.url_input.pop().is_some() {
            self.needs_repaint = true;
        }
    }

    pub fn handle_submit_url(&mut self) {
        let target = self.url_input.trim().to_string();
        self.url_focused = false;
        if target.is_empty() {
            self.url_input = self.active_url_str().to_string();
            self.needs_repaint = true;
        } else {
            self.navigate(&target);
        }
    }

    pub fn handle_key_navigation(&mut self, key: NavKey) {
        match key {
            NavKey::Up => self.scroll(-48.0),
            NavKey::Down => self.scroll(48.0),
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
                    self.url_focused = true;
                    self.needs_repaint = true;
                }
            }
            NavKey::Escape => {
                if self.url_focused {
                    self.url_focused = false;
                    self.url_input = self.active_url_str().to_string();
                    self.needs_repaint = true;
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
            NavKey::GoHome => self.navigate(HOME_URL),
        }
    }

    pub fn render_into(&mut self, canvas: &mut Canvas) {
        let tab_titles: Vec<&str> = self.tabs.iter().map(|t| t.title.as_str()).collect();
        let tab = &self.tabs[self.active_tab];
        let display_list = tab.display_list.as_deref().unwrap_or(&[]);
        let chrome = ChromeState {
            url_input: &self.url_input,
            url_focused: self.url_focused,
            can_back: !tab.back_stack.is_empty(),
            can_forward: !tab.forward_stack.is_empty(),
            status_text: &self.status_text,
            timing_badge: &tab.timing_badge,
            tab_titles: &tab_titles,
            active_tab: self.active_tab,
            scroll_y: tab.scroll_y,
            content_height: tab.content_height,
        };
        render_frame(canvas, &mut self.font_engine, display_list, &chrome);
    }

    pub fn render(&mut self) -> Canvas {
        let mut canvas = Canvas::new(
            self.viewport_width,
            self.viewport_height,
            Color::rgb(255, 255, 255),
        );
        self.render_into(&mut canvas);
        canvas
    }
}

impl PlatformApp for Browser {
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

    fn handle_key_navigation(&mut self, key: NavKey) {
        Browser::handle_key_navigation(self, key);
    }

    fn render_into(&mut self, canvas: &mut Canvas) {
        Browser::render_into(self, canvas);
    }
}

fn relayout_tab_state(tab: &mut TabState, ctx: &StyleContext, page_vh: f32) {
    let (Some(doc), Some(stylesheet)) = (tab.doc.as_ref(), tab.stylesheet.as_ref()) else {
        return;
    };
    let layout_root = compute_layout_with_ctx(doc, stylesheet, ctx, &tab.images);
    let display_list = build_display_list(&layout_root);
    let content_height = compute_content_height(&layout_root);
    let max_scroll = (content_height - page_vh).max(0.0);
    tab.scroll_y = tab.scroll_y.clamp(0.0, max_scroll);
    tab.content_height = content_height;
    tab.layout_root = Some(layout_root);
    tab.display_list = Some(display_list);
}

fn compute_layout_with_ctx(
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

fn compute_content_height(root: &LayoutBox) -> f32 {
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
        let mut browser = Browser::new("about:droplet", 1100, 760);
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
        let mut browser = Browser::new("about:droplet", 1024, 768);
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
        let mut browser = Browser::new("about:droplet", 1024, 768);
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
        let mut browser = Browser::new("about:roadmap", 900, 400);
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
        let mut browser = Browser::new("about:test", 1024, 768);
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
        let mut browser = Browser::new("about:droplet", 1024, 768);
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
        let mut browser = Browser::new(&url1, 800, 600);
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
        let mut browser = Browser::new("about:droplet", 1024, 768);
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
        let mut browser = Browser::new(&url, 600, 400);
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
        let mut browser = Browser::new(&static_url, 600, 400);
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
}
