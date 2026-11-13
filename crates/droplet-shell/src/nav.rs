//! Background navigation (DROP-110).
//!
//! Each navigation gets a process-unique `nav_id` and is recorded in its tab as
//! [`PendingNav`]. In async mode the page is fetched and parsed on a short-lived
//! `droplet-nav` thread; the result comes back over a channel and is committed only if
//! some tab still has that `nav_id` pending. Anything that supersedes a navigation (a new
//! navigation in the tab, `stop()`, closing the tab) just replaces or drops the pending
//! entry, so a late result is discarded: that is the whole cancellation mechanism.
//!
//! Without a waker (headless CLI, tests, platforms without a wake-up channel) navigation
//! runs inline on the caller's thread, exactly as before.

use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Instant;

use droplet_platform::Waker;
use droplet_url::Url;

use crate::loader::{self, LoadedPage};

/// How a committed navigation changes the tab's back/forward stacks.
///
/// Stacks change only on commit, so a cancelled navigation leaves history untouched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryOp {
    /// Reload, tab wake-up, first load: history unchanged.
    None,
    /// Regular navigation: push the current URL, clear forward.
    Push,
    /// The target is the top of the back stack.
    Back,
    /// The target is the top of the forward stack.
    Forward,
}

/// A navigation that has been started but not yet committed to its tab.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingNav {
    pub nav_id: u64,
    pub url: Url,
    pub history: HistoryOp,
    /// Scroll offset to restore after commit (waking a hibernated tab).
    pub restore_scroll: Option<f32>,
    pub start: Instant,
}

/// What to fetch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NavRequest {
    Get,
    /// `application/x-www-form-urlencoded` body.
    Post(String),
}

pub(crate) struct NavResult {
    pub nav_id: u64,
    pub page: LoadedPage,
}

pub(crate) struct Navigator {
    tx: Sender<NavResult>,
    rx: Receiver<NavResult>,
    waker: Option<Waker>,
    next_id: u64,
}

pub(crate) fn fetch_page(url: &Url, request: &NavRequest, js: bool) -> LoadedPage {
    match request {
        NavRequest::Get => loader::load_url_document(url, js),
        NavRequest::Post(body) => loader::load_url_document_post(url, body, js),
    }
}

impl Navigator {
    pub(crate) fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            tx,
            rx,
            waker: None,
            next_id: 0,
        }
    }

    pub(crate) fn set_waker(&mut self, waker: Waker) {
        self.waker = Some(waker);
    }

    pub(crate) fn next_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    /// Fetches on a background thread. Navigations go async only when the platform gave us
    /// a waker; otherwise (or if no thread could be spawned) the request is handed back so
    /// the caller loads inline.
    pub(crate) fn spawn(
        &self,
        nav_id: u64,
        url: Url,
        request: NavRequest,
        js: bool,
    ) -> Result<(), Box<(Url, NavRequest)>> {
        let Some(waker) = self.waker.clone() else {
            return Err(Box::new((url, request)));
        };
        let tx = self.tx.clone();
        let fallback = Box::new((url.clone(), request.clone()));
        std::thread::Builder::new()
            .name("droplet-nav".to_string())
            .spawn(move || {
                let page = fetch_page(&url, &request, js);
                // The receiver lives as long as the Browser; a send error means it is gone.
                if tx.send(NavResult { nav_id, page }).is_ok() {
                    waker();
                }
            })
            .map(|_| ())
            .map_err(|_| fallback)
    }

    pub(crate) fn try_recv(&self) -> Option<NavResult> {
        self.rx.try_recv().ok()
    }
}
