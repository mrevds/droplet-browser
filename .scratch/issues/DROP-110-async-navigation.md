# DROP-110 — Navigation off the UI thread

Phase 1 · Status: ready · Blocked by: — · Owner: _(write your name/model here when you start)_

## Problem
`Browser::load_url_into_tab` (`crates/droplet-shell/src/lib.rs`) calls `loader::load_url_document`
synchronously; `droplet_net::fetch_many` / `loader::load_document_images` use `std::thread::scope`,
which also blocks the caller. All three backends block in `XNextEvent` / `GetMessageW` /
`wl_display_dispatch`, so the window cannot even repaint while a page loads (2–8 s on HN/Wikipedia).

## Design
- `droplet-shell`: a `Navigator` owning one worker thread and an `mpsc` channel. A navigation request
  carries `(tab_idx, nav_id, Url, method/body, js)`; the worker runs the existing loader and sends
  back `LoadedPage` (it is `Send`; verify). The UI keeps `pending_nav_id` per tab; a result whose
  `nav_id` is stale (user navigated again / pressed Esc / closed tab) is dropped = cancellation.
- While loading: status text `Loading <host>…`, tab title unchanged, previous page stays
  interactive (scroll, tabs, address bar).
- `droplet-platform`: add `fn poll_background(&mut self) -> bool` (default `false`) to `PlatformApp`
  plus a cross-thread **waker** the shell can trigger after sending a result:
  - X11: `poll()` on `ConnectionNumber(display)` + a self-pipe/eventfd instead of bare `XNextEvent`;
  - Win32: `PostMessageW(hwnd, WM_APP, ..)` from the waker;
  - Wayland (frozen, minimal change allowed): `poll()` on `wl_display_get_fd` + the same pipe.
  The waker type must be OS-agnostic in the trait (`Arc<dyn Fn() + Send + Sync>`).
- Headless paths (`--screenshot`, `--dump-layout`, `--bench`, tests) keep a synchronous mode:
  `Browser::new` waits for the first navigation to finish, so goldens stay deterministic.

## Acceptance
- Unit test: start a navigation to a local `TcpListener` that sleeps 500 ms; `Browser` methods
  (`scroll`, `handle_text_input`, `render`) return immediately while it is pending; result applies after.
- Unit test: two navigations back-to-back → only the second page is applied (stale `nav_id` dropped).
- Unit test: Esc / `stop()` while pending → page unchanged, status cleared.
- Manual: GUI on HN — window repaints and scrolls while loading (record in the commit message).
- Corpus goldens unchanged; `--bench about:droplet` not worse; Cyrillic URL in the status line.
