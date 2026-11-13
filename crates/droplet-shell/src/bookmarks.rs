//! Ctrl+D bookmark toggle and its status-bar feedback.

use crate::Browser;

impl Browser {
    /// Toggles a bookmark for the active tab and reports the result in the status bar.
    pub(crate) fn toggle_current_bookmark(&mut self) {
        let url = self.active_url_str().to_string();
        let title = self.active_title().to_string();
        let added = self.store.toggle_bookmark(&url, &title);
        self.base_status_text = if added {
            "Bookmark added".to_string()
        } else {
            "Bookmark removed".to_string()
        };
        self.status_text = self.base_status_text.clone();
        self.needs_repaint = true;
    }
}
