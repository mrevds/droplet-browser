//! Shell-side history, bookmarks, generated user pages and address-bar
//! autocomplete. All disk failures are non-fatal.

use std::path::{Path, PathBuf};

use droplet_net::user_pages;
use droplet_storage::{now_timestamp, Bookmark, HistoryEntry, Store, Suggestion};
use droplet_url::Url;

use crate::loader::{self, LoadedPage};

const AUTOCOMPLETE_LIMIT: usize = 8;
const USER_PAGE_LIMIT: usize = 200;

/// One inline-completion candidate set for the current address-bar query.
#[derive(Debug, Default)]
struct Autocomplete {
    candidates: Vec<Suggestion>,
    selected: usize,
    active: bool,
}

/// Persistent history/bookmarks plus the address-bar suggestion state.
#[derive(Debug)]
pub struct HistoryStore {
    store: Store,
    autocomplete: Autocomplete,
}

impl HistoryStore {
    pub fn from_env() -> Self {
        Self::new(Store::from_env())
    }

    pub fn open(dir: impl Into<PathBuf>) -> Self {
        Self::new(Store::open(dir))
    }

    fn new(store: Store) -> Self {
        Self {
            store,
            autocomplete: Autocomplete::default(),
        }
    }

    pub fn data_dir(&self) -> &Path {
        self.store.data_dir()
    }

    pub fn record(&mut self, url: &str, title: &str) {
        let _ = self.store.add_history(url, title, now_timestamp());
    }

    /// Toggles the bookmark and returns whether the URL is bookmarked afterwards.
    pub fn toggle_bookmark(&mut self, url: &str, title: &str) -> bool {
        match self.store.toggle_bookmark(url, title, now_timestamp()) {
            Ok(added) => added,
            Err(_) => self.store.is_bookmarked(url),
        }
    }

    pub fn is_bookmarked(&self, url: &str) -> bool {
        self.store.is_bookmarked(url)
    }

    pub fn history(&self) -> &[HistoryEntry] {
        self.store.history()
    }

    pub fn bookmarks(&self) -> &[Bookmark] {
        self.store.bookmarks()
    }

    /// Serves `about:history` and `about:bookmarks` from this store, applying
    /// `?remove=` / `?clear=1` actions first. Returns `None` for other URLs.
    pub fn user_page(&mut self, url: &Url) -> Option<LoadedPage> {
        if !url.is_about() {
            return None;
        }
        let page = url.path.as_str();
        if !matches!(page, "history" | "bookmarks") {
            return None;
        }

        self.apply_query_actions(page, url.query.as_deref());

        let body = match page {
            "history" => user_pages::history_page(&self.store.recent_history(USER_PAGE_LIMIT)),
            _ => {
                let recent: Vec<_> = self
                    .store
                    .bookmarks()
                    .iter()
                    .rev()
                    .take(USER_PAGE_LIMIT)
                    .cloned()
                    .collect();
                user_pages::bookmarks_page(&recent)
            }
        };
        // Strip the action query so a reload does not repeat it.
        let clean = Url::parse(&format!("about:{page}")).ok()?;
        Some(loader::build_loaded_page(
            clean,
            &body,
            None,
            String::new(),
            false,
        ))
    }

    fn apply_query_actions(&mut self, page: &str, query: Option<&str>) {
        let Some(query) = query else {
            return;
        };
        for pair in query.split('&') {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            match (page, key) {
                ("history", "remove") => {
                    if let Some(url) = user_pages::percent_decode(value) {
                        let _ = self.store.remove_history(&url);
                    }
                }
                ("history", "clear") if value == "1" => {
                    let _ = self.store.clear_history();
                }
                ("bookmarks", "remove") => {
                    if let Some(url) = user_pages::percent_decode(value) {
                        let _ = self.store.remove_bookmark(&url);
                    }
                }
                ("bookmarks", "clear") if value == "1" => {
                    let _ = self.store.clear_bookmarks();
                }
                _ => {}
            }
        }
    }

    pub fn refresh_autocomplete(&mut self, typed: &str) {
        self.autocomplete.candidates = self.store.suggest(typed, AUTOCOMPLETE_LIMIT);
        self.autocomplete.selected = 0;
        self.autocomplete.active =
            !typed.trim().is_empty() && !self.autocomplete.candidates.is_empty();
    }

    /// Inline suffix for the currently selected candidate (empty when none).
    pub fn completion_suffix(&self, typed: &str) -> String {
        if !self.autocomplete.active || typed.trim().is_empty() {
            return String::new();
        }
        let Some(candidate) = self.autocomplete.candidates.get(self.autocomplete.selected) else {
            return String::new();
        };
        suffix_after_prefix(&candidate.url, typed)
    }

    /// Moves the selected suggestion up/down with wrap-around and returns the URL.
    pub fn cycle_autocomplete(&mut self, typed: &str, forward: bool) -> Option<String> {
        if self.autocomplete.candidates.is_empty() {
            self.refresh_autocomplete(typed);
        }
        let len = self.autocomplete.candidates.len();
        if len == 0 {
            return None;
        }
        self.autocomplete.selected = if forward {
            (self.autocomplete.selected + 1) % len
        } else {
            (self.autocomplete.selected + len - 1) % len
        };
        self.autocomplete.active = true;
        Some(self.autocomplete.candidates[self.autocomplete.selected].url.clone())
    }

    /// Accepts the inline suggestion; returns `None` when there is nothing to add.
    pub fn accept_autocomplete(&mut self, typed: &str) -> Option<String> {
        if self.completion_suffix(typed).is_empty() {
            self.dismiss_autocomplete();
            return None;
        }
        let url = self
            .autocomplete
            .candidates
            .get(self.autocomplete.selected)
            .map(|candidate| candidate.url.clone());
        self.dismiss_autocomplete();
        url
    }

    pub fn dismiss_autocomplete(&mut self) {
        self.autocomplete.candidates.clear();
        self.autocomplete.selected = 0;
        self.autocomplete.active = false;
    }
}

fn suffix_after_prefix(candidate: &str, typed: &str) -> String {
    let prefix = typed.trim();
    if prefix.is_empty() || candidate.len() <= prefix.len() {
        return String::new();
    }
    let head = &candidate.as_bytes()[..prefix.len()];
    if head.eq_ignore_ascii_case(prefix.as_bytes()) {
        candidate[prefix.len()..].to_string()
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static SEQ: AtomicUsize = AtomicUsize::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            let seq = SEQ.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "droplet_shell_store_{label}_{}_{seq}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn autocomplete_suffix_and_cycle_are_prefix_based() {
        let dir = TempDir::new("autocomplete");
        let mut store = HistoryStore::open(&dir.0);
        store.record("https://example.com/alpha", "Alpha");
        store.record("about:test", "Test");

        store.refresh_autocomplete("https://example.com/a");
        assert_eq!(store.completion_suffix("https://example.com/a"), "lpha");
        assert_eq!(
            store.cycle_autocomplete("https://example.com/a", false),
            Some("https://example.com/alpha".to_string())
        );
        assert_eq!(
            store.accept_autocomplete("https://example.com/a"),
            Some("https://example.com/alpha".to_string())
        );
        assert_eq!(store.completion_suffix("https://example.com/a"), "");

        store.refresh_autocomplete("ABOUT:");
        assert_eq!(store.completion_suffix("ABOUT:"), "test");
        assert_eq!(
            store.accept_autocomplete("ABOUT:"),
            Some("about:test".to_string())
        );
    }

    #[test]
    fn user_pages_apply_remove_and_clear_actions() {
        let dir = TempDir::new("actions");
        let mut store = HistoryStore::open(&dir.0);
        store.record("about:test", "Test");
        store.record("about:roadmap", "Roadmap");
        store.toggle_bookmark("about:test", "Test");

        let history_url =
            Url::parse("about:history?remove=about%3Atest").expect("history action URL");
        let page = store.user_page(&history_url).expect("history page");
        assert_eq!(page.final_url.as_str(), "about:history");
        assert!(!page.doc.dump_tree().contains("remove=about%3Atest"));

        let clear = Url::parse("about:history?clear=1").expect("clear URL");
        store.user_page(&clear).expect("history page");
        let page = store.user_page(&Url::parse("about:history").unwrap()).unwrap();
        assert!(page.doc.dump_tree().contains("No history yet"));

        let remove_bookmark =
            Url::parse("about:bookmarks?remove=about%3Atest").expect("bookmark action URL");
        store.user_page(&remove_bookmark).expect("bookmarks page");
        assert!(!store.is_bookmarked("about:test"));
    }
}
