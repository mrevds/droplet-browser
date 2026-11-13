# DROP-006: Persistent History/Bookmarks, Find-in-Page & Box-Model DevTools

- **Phase:** 6
- **Status:** `in-progress`
- **Blocked By:** `DROP-002`, `DROP-004` (both `done`)
- **Implementation owner:** DeepSeek V4.1 Flash Free (OpenCode agent)

---

## Goal

Turn the multi-tab shell into a daily-driver: persistent history/bookmarks,
prefix autocomplete in the address bar, find-in-page with highlights, keyboard
tab switching, and a box-model inspector overlay — zero dependencies.

## Required scope

1. **Storage crate** — new `crates/droplet-storage` with a compact append-log or
   length-prefixed binary store. Versioned header, UTF-8 safe escaping, capped
   size/entry count, tolerant of truncated/corrupt tails. Unit tests use unique
   temp directories and cover round-trip, corruption, capping and escaping.
2. **History** — record successful navigations (URL + title + timestamp), expose
   recent history, clear history. Persist under a platform-neutral data path with
   an injectable path for tests (`DROPLET_DATA_DIR` override).
3. **Bookmarks** — toggle bookmark for current page (`Ctrl+D`), list bookmarks,
   persist, remove entries. `about:history` and `about:bookmarks` pages render
   the stored lists and link back.
4. **Address bar autocomplete** — prefix match over history + bookmarks; show the
   completion inline and accept with `Tab`/`Right`; `Up`/`Down` cycle candidates.
5. **Find-in-page** — `Ctrl+F` opens a compact find bar in chrome, type query,
   `Enter`/`F3` next, `Shift+Enter`/`Shift+F3` previous, `Esc` closes. Highlight
   all matches and the active match. Count indicator (e.g. `3/17`). Must be
   UTF-8 safe and case-insensitive (ASCII + Cyrillic).
6. **Keyboard-first shell** — `Ctrl+1..9` switches to tab N and `Ctrl+Tab` /
   `Ctrl+Shift+Tab` remain intact.
7. **Box-model inspector** — `Ctrl+Shift+I` toggles an overlay that highlights
   the hovered layout box: content, padding, border, margin rectangles with
   distinct colors and a compact label (`tag`, `NodeId`, size).
8. **Tests** — storage tests, history/bookmark navigation tests, find match tests
   (UTF-8/Cyrillic, wrap-around), overlay geometry tests, and a headless check
   that `about:history` / `about:bookmarks` render.

## Constraints

- Zero external dependencies; no SQLite.
- Do not block the UI thread on disk I/O for long; writes are small append-log
  operations, and failures must never panic the browser.
- Use `droplet-paint`'s generic overlay API for highlights and inspector rects.
- Keep tab hibernation working; stores are shared across tabs, not per-tab.
