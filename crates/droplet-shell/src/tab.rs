use std::collections::HashMap;
use std::sync::Arc;

use droplet_dom::Document;
use droplet_image::DecodedImage;
use droplet_layout::LayoutBox;
use droplet_paint::DisplayList;
use droplet_style::Stylesheet;
use droplet_url::Url;

use crate::forms::FormState;
use crate::js::TabJsState;
use crate::nav::PendingNav;

/// Maximum number of non-hibernated tabs kept warm in memory simultaneously.
pub const MAX_WARM_TABS: usize = 3;

/// Granular per-tab dirty tracking so idle ticks and paint-only edits avoid
/// redundant styling and layout passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DirtyFlags {
    pub style_and_layout: bool,
    pub paint: bool,
    pub title_changed: bool,
}

impl DirtyFlags {
    pub const fn clean() -> Self {
        Self {
            style_and_layout: false,
            paint: false,
            title_changed: false,
        }
    }

    pub const fn is_clean(self) -> bool {
        !self.style_and_layout && !self.paint && !self.title_changed
    }

    pub fn mark_paint(&mut self) {
        self.paint = true;
    }

    pub fn mark_style_and_layout(&mut self) {
        self.style_and_layout = true;
        self.paint = true;
    }

    pub fn mark_title(&mut self) {
        self.title_changed = true;
    }

    pub fn take(&mut self) -> Self {
        let taken = *self;
        *self = Self::clean();
        taken
    }
}

/// Per-tab navigation, render payload, and hibernation state.
#[derive(Debug)]
pub struct TabState {
    pub current_url: Url,
    pub title: String,
    pub doc: Option<Document>,
    pub stylesheet: Option<Stylesheet>,
    pub uses_interaction: bool,
    pub images: HashMap<String, Arc<DecodedImage>>,
    pub layout_root: Option<LayoutBox>,
    pub display_list: Option<DisplayList>,
    pub js: TabJsState,
    pub back_stack: Vec<Url>,
    pub forward_stack: Vec<Url>,
    pub scroll_y: f32,
    pub content_height: f32,
    pub timing_badge: String,
    pub hibernated: bool,
    pub last_active_seq: u64,
    /// Per-tab focus/caret/selection state (never shared across tabs).
    pub forms: FormState,
    /// Pending DOM/style/paint invalidation flags.
    pub dirty: DirtyFlags,
    /// Number of full style+layout passes run for this tab since load/reset.
    pub layout_rebuilds: usize,
    /// Number of display-list rebuilds run for this tab since load/reset.
    pub display_list_rebuilds: usize,
    /// Navigation started but not yet committed (DROP-110). The visible page stays the
    /// previous one until the result arrives.
    pub pending: Option<PendingNav>,
    /// Out-of-process WebKit view showing this tab instead of Droplet's engine (DROP-170).
    pub webview: Option<crate::webview::WebView>,
}

impl TabState {
    /// Creates an empty tab initialized to `about:blank` with the given title and LRU sequence.
    pub fn create_tab(title: impl Into<String>, last_active_seq: u64) -> Self {
        let placeholder_url = Url::parse("about:blank").expect("about:blank always parses");
        Self {
            current_url: placeholder_url,
            title: title.into(),
            doc: None,
            stylesheet: None,
            uses_interaction: false,
            images: HashMap::new(),
            layout_root: None,
            display_list: None,
            js: TabJsState::default(),
            back_stack: Vec::new(),
            forward_stack: Vec::new(),
            scroll_y: 0.0,
            content_height: 0.0,
            timing_badge: String::new(),
            hibernated: false,
            last_active_seq,
            forms: FormState::default(),
            dirty: DirtyFlags::clean(),
            layout_rebuilds: 0,
            display_list_rebuilds: 0,
            pending: None,
            webview: None,
        }
    }

    /// Drops heavy parsed DOM, stylesheet, decoded images, layout tree, display list,
    /// and JS runtime state, marking the tab as hibernated while retaining its URL, title,
    /// and navigation stacks.
    pub fn clear_warm_payload(&mut self) {
        self.webview = None; // ends the helper process and frees its memory
        self.doc = None;
        self.stylesheet = None;
        self.images.clear();
        self.images.shrink_to_fit();
        self.layout_root = None;
        self.display_list = None;
        self.js.reset();
        self.forms.reset();
        self.dirty = DirtyFlags::clean();
        self.hibernated = true;
    }

    /// Resets navigation history and scroll position when recycling the sole remaining tab.
    pub fn reset_for_reuse(&mut self, last_active_seq: u64) {
        self.pending = None;
        self.back_stack.clear();
        self.forward_stack.clear();
        self.scroll_y = 0.0;
        self.js.reset();
        self.hibernated = false;
        self.last_active_seq = last_active_seq;
        self.forms.reset();
        self.dirty = DirtyFlags::clean();
        self.layout_rebuilds = 0;
        self.display_list_rebuilds = 0;
    }
}
