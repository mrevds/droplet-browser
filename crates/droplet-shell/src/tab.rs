use std::collections::HashMap;
use std::sync::Arc;

use droplet_dom::Document;
use droplet_image::DecodedImage;
use droplet_layout::LayoutBox;
use droplet_paint::DisplayList;
use droplet_style::Stylesheet;
use droplet_url::Url;

/// Maximum number of non-hibernated tabs kept warm in memory simultaneously.
pub const MAX_WARM_TABS: usize = 3;

/// Per-tab navigation, render payload, and hibernation state.
#[derive(Debug, Clone)]
pub struct TabState {
    pub current_url: Url,
    pub title: String,
    pub doc: Option<Document>,
    pub stylesheet: Option<Stylesheet>,
    pub uses_interaction: bool,
    pub images: HashMap<String, Arc<DecodedImage>>,
    pub layout_root: Option<LayoutBox>,
    pub display_list: Option<DisplayList>,
    pub back_stack: Vec<Url>,
    pub forward_stack: Vec<Url>,
    pub scroll_y: f32,
    pub content_height: f32,
    pub timing_badge: String,
    pub hibernated: bool,
    pub last_active_seq: u64,
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
            back_stack: Vec::new(),
            forward_stack: Vec::new(),
            scroll_y: 0.0,
            content_height: 0.0,
            timing_badge: String::new(),
            hibernated: false,
            last_active_seq,
        }
    }

    /// Drops heavy parsed DOM, stylesheet, decoded images, layout tree, and display list,
    /// marking the tab as hibernated while retaining its URL, title, and navigation stacks.
    pub fn clear_warm_payload(&mut self) {
        self.doc = None;
        self.stylesheet = None;
        self.images.clear();
        self.images.shrink_to_fit();
        self.layout_root = None;
        self.display_list = None;
        self.hibernated = true;
    }

    /// Resets navigation history and scroll position when recycling the sole remaining tab.
    pub fn reset_for_reuse(&mut self, last_active_seq: u64) {
        self.back_stack.clear();
        self.forward_stack.clear();
        self.scroll_y = 0.0;
        self.hibernated = false;
        self.last_active_seq = last_active_seq;
    }
}
