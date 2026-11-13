# DROP-110 — Navigation off the UI thread

Phase 1 · Status: done · Blocked by: — · Owner: Claude Opus 5.5 (Claude Code)

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

## Result (2026-11-13)
- `crates/droplet-shell/src/nav.rs`: `Navigator` (thread per navigation, `mpsc` results), `PendingNav`,
  `HistoryOp`, `NavRequest`. `Browser::start_navigation` / `commit_navigation` / `poll_navigations` /
  `stop` / `is_loading` / `new_deferred`. Cancellation = the tab no longer holds that `nav_id`.
- Deviation from the spec: async mode switches on when the platform calls `set_waker` (no explicit
  sync flag). Headless paths and tests never get a waker and stay synchronous, so goldens are unchanged.
- `crates/droplet-platform`: `Waker`, `PlatformApp::{set_waker, poll_background}`, `linux_wake.rs`
  (self-pipe + `poll`), X11 and Wayland loops wait on display fd + pipe, Win32 `WM_APP + 1` via
  `PostMessageW`. macOS untouched (no waker → synchronous).
- Tests: 6 async scenarios (responsiveness, superseded result, Esc stop, closing a loading tab,
  back/forward stacks on commit, inactive-tab load), 2 deferred-start tests, wake-pipe test;
  async tests ran 10/10 times green.
- Manual GUI check (X11 window title via `xwininfo`; Wayland via thread lifecycle):

  | site | window shown | page committed | idle CPU after |
  | --- | --- | --- | --- |
  | example.com (X11) | 117 ms | 372 ms | 0 ticks / 2 s |
  | rust-lang.org (X11) | 118 ms | 2449 ms | 0 ticks / 2 s |
  | news.ycombinator.com (X11) | 121 ms | 2593 ms | 0 ticks / 2 s |
  | rust-lang.org (Wayland) | 119 ms | nav threads done at 2743 ms | 0 ticks / 2 s |
  | news.ycombinator.com (Wayland) | 119 ms | still in TLS handshake after 6 s (DROP-111) — UI idle, not frozen | 0 ticks / 2 s |

- Found while verifying, fixed in a separate commit: the X11 MIT-SHM backend crashed with
  `BadShmSeg` on the first `XShmPutImage` (also on `main`), because `XShmCreateImage` keeps a
  pointer to a stack `XShmSegmentInfo` that was then moved. It was hidden because Wayland is
  preferred and the old synchronous first load delayed the first blit.
- Not done: Win32 path compiled and clippy-checked for `x86_64-pc-windows-gnu` only, not run on Windows.
