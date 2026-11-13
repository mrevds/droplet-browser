# Droplet Engine Architecture

**Droplet** is a zero-bloat graphical web browser engine written from scratch in pure Rust (`edition = "2021"`).  
All workspace `[dependencies]` stay **completely empty** of external crates—zero async runtimes, zero proc-macro graphs, and zero build-time C/C++ toolchains. OS integration relies strictly on Rust `std` plus runtime dynamic library loading (`dlopen` on Linux for `libX11.so.6`, `libXext.so.6`, `libwayland-client.so.0`, and `libssl.so.4`/`.3`/`.so`), Win32/Schannel/GDI FFI on Windows, and Objective-C/CoreGraphics FFI on macOS.

---

## End-to-End Rendering Pipeline

```text
  Input URL / CLI / Click / Tab Switch / JS Event
           │
           ▼
   [1] crates/droplet-url/src/lib.rs          Url::parse / Url::resolve (http://, https://, file://, about:*)
           │
           ▼
   [2] crates/droplet-net/src/adblock.rs      TrackerFilter::should_block (zero-alloc domain/suffix check before DNS/TCP)
     + crates/droplet-net/src/{lib,tls,tls_schannel}.rs
                                              DNS TTL cache -> HTTP/1.1 Keep-Alive pool -> OpenSSL (Linux) / Schannel (Windows)
                                              fetch_many (4-worker std::thread pool) / fetch_uri_bytes (data:, file:, http(s):)
                                              Built-in about:* & generated about:history / about:bookmarks pages
           │
           ▼
   [3] crates/droplet-dom/src/{html,arena,query}.rs
                                              html::parse -> Arena Document (Vec<Node>, NodeId = usize, Vec<(String, String)> attrs)
                                              Iterative DOM mutation APIs & CSS selector queries (query_selector / query_selector_all)
           │
           ▼
   [4] crates/droplet-js/src/                 Bytecode VM + DomHost bindings (<script> execution, timers, events,
     + crates/droplet-shell/src/js.rs         document.getElementById / querySelector / classList / style / ShellDomHost)
           │
           ▼
   [5] crates/droplet-image/src/              image::decode -> DecodedImage (pure-Rust PNG, baseline JPEG, BMP, PPM, SVG;
                                              64M-px dimension-bomb guard)
           │
           ▼
   [6] crates/droplet-style/src/              css::parse -> style_tree_with_context (Copy ComputedStyle, zero HashMap, StyleContext)
           │
           ▼
   [7] crates/droplet-layout/src/             layout_tree (Block, Inline, InlineBlock, 2-pass Flexbox, <img>, HTML form controls)
                                              hit_test / hit_test_href / hit_test_node
           │
           ▼
   [8] crates/droplet-paint/src/              build_display_list -> DisplayList -> render_frame_damaged -> Canvas / External Slice
     + crates/droplet-font/src/               FontEngine (Linux/Windows TTF discovery, pure-Rust sfnt/glyf AA,
                                              bounded GlyphAtlas max 4096 entries, 8x16 bitmap fallback)
           │
           ├──────────────────────────────────────────────────────┐
           ▼                                                      ▼
  [9a] crates/droplet-platform/src/                        [9b] CLI & Multi-Process Sandbox (`apps/droplet-desktop/`)
       lib.rs -> linux_wayland / linux_x11 /                    --dump-dom / --dump-layout / --screenshot
                 windows / macos + clipboard                    --sandbox-screenshot -> RendererHost <-> --renderer child
                                                                (crates/droplet-ipc + crates/droplet-sandbox + HeadlessRenderer)
```

The orchestration layer (`crates/droplet-shell/src/` -> `Browser`) manages multi-tab `TabState` slots, automatic **Tab Hibernation** past `MAX_WARM_TABS = 3`, persistent history/bookmarks (`crates/droplet-storage`), interactive HTML forms & selection, Find-in-Page, Box Model Inspector, reactive `TabJsState` + `DirtyFlags` incremental style/layout/display-list rebuilds, and `DamageRect` partial-surface dirty tracking.

---

## Module & Data Structure Reference

### 1. URL Parser & Resolver (`crates/droplet-url/src/lib.rs`)
- `Scheme`: `Http | Https | File | About`
- `Url`: `{ raw: String, scheme: Scheme, host: String, port: u16, path: String, query: Option<String>, fragment: Option<String> }`
- `UrlError`: `{ message: String }`
- `Url::parse` normalizes bare domain inputs (`example.com` -> `http://example.com/`), bare paths (`/foo.html` -> `file:///foo.html`), `https://` (default port `443`), `http://` (default port `80`), and `about:*`.
- `Url::resolve` handles `#fragment`, explicit schemes, protocol-relative `//host/path` (inheriting `http:` or `https:` from base), root-relative `/path`, query-only `?q=...`, and dot-segment normalization (`.`, `..`).

### 2. Zero-Allocation Tracker Filter (`crates/droplet-net/src/adblock.rs`)
- `TrackerFilter`: `{ defaults: &'static [&'static str], custom: Vec<String> }`.
- `should_block(host)` trims trailing dots, walks domain labels from full host down to parent suffixes (`sub.tracker.net` -> `tracker.net`), and binary-searches sorted domain tables using case-insensitive ASCII comparison (`cmp_ascii_ignore_case`).
- Performs **zero heap allocations** on the hot check path and executes **before** DNS lookup or `TcpStream` connect so blocked tracker/ad subresources never open sockets or burn CPU/bandwidth.
- Label-boundary safe (`notdoubleclick.net` is not matched by `doubleclick.net`).

### 3. Transport, Pooling, TLS & Built-in Pages (`crates/droplet-net/src/`)
- `Response`: `{ url: Url, status: u16, content_type: String, body: String }`, `NetError`: `{ message: String }`.
- **Public API:** `fetch`, `fetch_with_filter`, `fetch_many`, `fetch_many_with_filter`, `fetch_bytes`, `fetch_uri_bytes` (`data:`, `file:`, `http:`, `https:`), plus pure parsers `parse_http_response` and `decode_chunked`.
- **Caching & Pooling (`dns.rs`, `pool.rs`):**
  - **DNS cache:** `(host, port) -> DnsEntry` with `DNS_TTL = 60s` and `DNS_CACHE_MAX_ENTRIES = 256`.
  - **HTTP/1.1 Keep-Alive connection pool:** keyed by `(SchemeKey, host, port)` with `POOL_IDLE_TTL = 30s`, `POOL_MAX_PER_ORIGIN = 4`, and `POOL_MAX_TOTAL = 32`. Reads framed `Content-Length` and incremental `Transfer-Encoding: chunked` bodies, retrying once on stale pooled sockets.
- **TLS (`tls.rs`, `tls_schannel.rs`):**
  - On **Linux**, `tls::TlsStream` dynamically loads `libssl.so.4`, `libssl.so.3`, or `libssl.so` at runtime via `dlopen`, configures system CA verification (`SSL_CTX_set_default_verify_paths` + `SSL_VERIFY_PEER`), sets SNI (`SSL_set_tlsext_host_name`), and verifies the peer hostname (`SSL_set1_host` + `SSL_get_verify_result`).
  - On **Windows**, `tls_schannel::TlsStream` uses native Win32 **Schannel** (`secur32.dll` / `crypt32.dll`: `AcquireCredentialsHandleW`, `InitializeSecurityContextW`, `QueryContextAttributesW`, `EncryptMessage` / `DecryptMessage`, `FreeCredentialsHandle`) with system certificate trust and SNI—without OpenSSL.
  - On unsupported targets, `tls::TlsStream::connect` returns a descriptive `NetError`.
- **Concurrency & Limits:** `fetch_many` uses a bounded `std::thread::scope` pool (`MAX_CONCURRENT_FETCHES = 4`); enforces `CONNECT_TIMEOUT = 10 s` (all resolved addresses), `HANDSHAKE_TIMEOUT = 15 s` (total, retried across socket waits — DROP-111), `READ_TIMEOUT = 15 s` idle, `MAX_REDIRECTS = 5`, `MAX_HEADER_BYTES = 64 KiB`, and `MAX_RESPONSE_BYTES = 16 MiB`.
- **Built-in & User Pages (`about.rs`, `user_pages.rs`):** `about:droplet`, `about:roadmap`, `about:architecture`, `about:test`, `about:blank`, plus dynamic HTML views for `about:history` and `about:bookmarks`.

### 3b. DEFLATE / zlib / gzip (`crates/droplet-inflate/src/lib.rs`, DROP-112)
- Dependency-free crate shared by `droplet-image` (PNG `IDAT`) and `droplet-net` (`Content-Encoding`): `inflate_raw`, `inflate_zlib` (Adler-32), `gunzip` (RFC 1952 header flags, multi-member, CRC-32 + ISIZE per member, NUL padding tolerated), `looks_like_zlib`, table-driven `crc32`. Every call takes `max_out` and fails *before* producing bytes past it, so a decompression bomb never allocates past the limit (`MAX_RESPONSE_BYTES = 16 MiB` for HTTP). Huffman decoding is bit-at-a-time: ~5.6 ms for the 1 MB Wikipedia article (C zlib: ~2.4 ms).
- `droplet-net` sends `Accept-Encoding: gzip, deflate` and undoes `Content-Encoding` right to left after de-chunking (`gzip`/`x-gzip`, `deflate` as zlib or raw, `identity`); an unknown encoding or a corrupt stream is a load error, never rendered as text.

### 3c. Cookies (`crates/droplet-net/src/cookies.rs`, DROP-113)
- RFC 6265 subset: `Set-Cookie` parsing (§5.2; `Max-Age` beats `Expires`, §5.1.1 cookie-date algorithm), `Domain` (host-only vs domain match; single-label domains such as `com`/`рф` rejected unless equal to the host; IP hosts match exactly), `Path` (default path + path match), `Secure` (set and sent only over `https`; plain `http` cannot overwrite a `Secure` cookie — RFC 6265bis), `HttpOnly` and `SameSite` parsed and stored (no JS access exists; `SameSite` is not enforced yet). Hosts are compared Unicode-lowercased, no punycode. Ports are ignored, as in browsers.
- `CookieJar` = `HashMap<domain, Vec<Cookie>>`; a request looks up only the host and its parent-domain buckets. Hard caps `MAX_COOKIES_PER_DOMAIN = 50`, `MAX_COOKIES_TOTAL = 3000`, `MAX_COOKIE_BYTES = 4096` (name + value); expired cookies are purged first, then the least recently used (stored or sent) one.
- One process-wide jar behind a `Mutex`. Every HTTP hop of `fetch*` (redirects and subresources included) sends `cookies::header_for(url)` as `Cookie:` and applies the response's `Set-Cookie` lines before following `Location`.
- Persistence: `Browser` construction calls `cookies::attach_store(data_dir)`; the loader calls `cookies::flush()` after each navigation (on the `droplet-nav` thread when backgrounded), which rewrites the file only if a persistent cookie changed. Session cookies are never written. File `cookies.txt` (via `droplet_storage::{load,save}_cookie_file`: temp file + rename, `0600` on Unix, ignored above 16 MiB):
  ```text
  DRPLCOOKIE1
  domain \t host_only \t path \t name \t value \t expires \t secure \t http_only \t samesite \t creation
  ```
  Text fields use `escape_field` (UTF-8 kept, controls escaped), flags `0`/`1`, times Unix seconds; malformed or expired lines are skipped on load.

### 4. Pure-Rust Image Decoder (`crates/droplet-image/src/`)
- `DecodedImage`: `{ pub width: u32, pub height: u32, pub rgba: Vec<u8> }` (row-major RGBA8); `ImageError`: `{ pub message: String }`.
- `decode(&[u8]) -> Result<DecodedImage, ImageError>` sniffs magic bytes and decodes in pure Rust (zero external crates):
  - **PNG (`png.rs` + `droplet-inflate`):** full chunk reader, DEFLATE/zlib inflate, all 5 scanline predictors (`None`/`Sub`/`Up`/`Average`/`Paeth`), Adam7 interlacing, grayscale/RGB/RGBA/indexed `PLTE` + `tRNS`.
  - **Baseline JPEG (`jpeg.rs`):** baseline DCT (`SOF0`) decoder supporting Huffman tables (`DHT`), quantization tables (`DQT`), grayscale and `4:2:0` / `4:4:4` chroma subsampling, `DRI` restart intervals, YCbCr -> RGBA8 conversion, clean rejection of progressive `SOF2` streams, and truncation checks.
  - **BMP (`bmp.rs`), PPM (`ppm.rs`):** uncompressed/RLE BMP and ASCII `P3` / binary `P6` PPM.
  - **Static SVG (`svg.rs`):** XML preamble + `<svg>` (`viewBox`, `<rect>`, `<circle>`, `<line>`, `<polygon>`, `<path>` fill/stroke rasterization).
- **Dimension Bomb Guard:** `validate_dimensions` enforces `width * height <= MAX_IMAGE_PIXELS` (`64_000_000`) and checked byte-size math before allocating `Vec<u8>`.

### 5. Arena DOM, HTML Parser & Selector Engine (`crates/droplet-dom/src/`)
- `NodeId = usize`; `ElementData`: `{ pub tag_name: String, pub attributes: Vec<(String, String)> }`. Storing attributes in a flat `Vec` instead of `HashMap` incurs **zero heap allocation** on attribute-less elements.
- `NodeKind`: `Document | Element(ElementData) | Text(String)`; `Document`: `{ nodes: Vec<Node> }` with `nodes[0]` always `NodeKind::Document`.
- Iterative `Vec`-stack traversals (zero recursion): `text_content`, `text_content_of`, `set_text_content_of`, `collect_style_text`, `collect_stylesheet_links`, `collect_script_sources`, `find_title`, `find_first_tag`, `dump_tree`.
- Live DOM mutation & query (`arena.rs`, `query.rs`): iterative `append_child` (with ancestor cycle check and reparenting), `remove_child`, `set_inner_html_fragment` / `serialize_inner_html`, `class_list_*`, `style_prop_get` / `style_prop_set` (`camelCase` <-> `kebab-case`), and iterative `get_element_by_id` / `query_selector` / `query_selector_all` (tag, `#id`, `.class`, `[attr]`, comma lists, descendant & `>` child combinators).
- `html::parse(&str) -> Document`: iterative UTF-8-safe HTML5-style stack parser handling `<!DOCTYPE>`, comments, void tags, self-closing tags, raw text (`<style>`, `<script>`), HTML entities, and implicit auto-closing rules.

### 6. Embedded JavaScript VM & Reactive DOM (`crates/droplet-js/src/`, `crates/droplet-shell/src/js.rs`) — frozen, off by default
- **Off by default.** `BrowserOptions { js }` (set by `droplet_shell::set_js_default`, i.e. the `--js` flag) decides whether the loader collects/fetches `<script>` at all. With JS off no script bytes are downloaded. The crate is frozen until the Phase 7 decision (see ROADMAP).
- **Pure-Rust Bytecode VM (`crates/droplet-js`):** lexer (`lexer.rs`), recursive-descent AST parser (`parser.rs`, `ast.rs`), bytecode compiler (`compiler.rs`, `bytecode.rs`), and stack VM (`vm.rs`) supporting variables (`let`/`const`/`var`), functions & closures, objects, arrays, host bindings (`HostBinding`), `console.log`, and `setTimeout` / `clearTimeout` / `setInterval` timers.
- **DOM Bridge (`dom.rs` + `ShellDomHost`):** exposes `DomNodeId`, `DomHost`, `DomEventInput`, `DomEventOutcome`, `MAX_DOM_LISTENERS = 1024`, `MAX_SCRIPTS_PER_PAGE = 32`, and `MAX_JS_LOG_ENTRIES = 256` across `document`, `window`, element wrappers (cached by `Rc` identity), `textContent`, `innerHTML`, `classList`, `style`, `value`, `checked`, `focus()`/`blur()`, and bubbling DOM event dispatch (`click`, `input`, `change`, `submit`, `keydown`, `keyup`) with `preventDefault()` and `stopPropagation()`.
- **Incremental Dirty Tracking (`crates/droplet-shell/src/tab.rs`):** `DirtyFlags` separates `paint`, `style_and_layout`, and `title_changed` so paint-only mutations avoid full relayout, tracked via `layout_rebuilds` and `display_list_rebuilds`.

### 7. CSS Parser & Style Tree (`crates/droplet-style/src/`)
- **CSS Parser (`css.rs`):** `Color`, `Unit` (`Px | Percent | Em | Rem | Vh | Vw`), `Value`, `CompoundSelector` (`tag_name`, `id`, `classes`, `universal`, `hover`, `active`, `focus`, `attrs: Vec<AttrSelector>`, `pseudo: Vec<PseudoClass>`), `ComplexSelector` (`Descendant` / `Child` / `NextSibling` `+` / `SubsequentSibling` `~`), `Rule`, `Stylesheet`. Selectors (DROP-123): attribute operators `[a] = ~= |= ^= $= *=` with `i`/`s` flags; `:root`, `:empty`, `:first/last/only-child`, `:nth-child`/`:nth-last-child` (`An+B`, `odd`/`even`, `of S`), `:first/last/only-of-type`, `:nth(-last)-of-type`; `:not()`, `:is()`, `:where()` (forgiving lists, nesting capped at 8). Level 4 specificity. Unknown pseudo-classes parse but never match; invalid syntax drops only that selector.
- **Style Tree (`cascade.rs`, `computed.rs`):**
  - `Display`: `Block | Inline | InlineBlock | Flex | None`.
  - `ComputedStyle`: derives `Copy` with **zero `HashMap`s** or heap allocations—only typed scalar/enum/edge fields including Flexbox properties, `box_sizing`, sizes as `LengthPct` (`Auto | Px | Percent`: `width`, `height`, `min-/max-width/height`) and `inset` (`top/right/bottom/left`, stored for positioned layout). Shorthand parsing (`padding`, `border*`, `background`, `font`, `inset`) lives in `shorthand.rs` and is all-or-nothing per declaration.
  - `StyleContext`: `{ viewport_width, viewport_height, hovered_node, active_node }` for `vh`/`vw` and `:hover`/`:active` matching; `stylesheet_uses_interaction` skips restyle/relayout on mouse motion for static pages.
  - **Selector matching (`rule_index.rs`, DROP-120):** `RuleIndex` files each selector under its rightmost compound's id, else first class, else tag, else a universal list; per element only rules from its own buckets are matched (results identical to testing every rule). `AncestorBloom`, a 4 KiB counting Bloom filter of ancestor tag/id/class keys kept by the depth-first style walk, rejects descendant/child selectors whose required ancestor keys are absent (compounds left of `+`/`~` are siblings and contribute no ancestor keys). A rightmost compound without id/class/tag is filed by a rare attribute name (`span[rel]` goes there before its tag; a 64-bit name mask skips most hash lookups), as `:root`, or under every argument of `:is()`/`:where()`. Built per style pass (UA index cached once); no per-node memory. `sibling_index.rs` caches element/type positions for structural selectors per parent, filled on first query, so `:nth-child` on long lists stays O(n).

### 8. Box Layout Engine & Forms (`crates/droplet-layout/src/`)
- `Rect` / `Dimensions` (`content`, `padding`, `border`, `margin`), `BoxType`, `LayoutBox`.
- `layout_tree` resolves block widths (`box-sizing`, then `max-*`, then `min-*` via `box_model::resolve_width/height` and `clamp_width/height`), auto margins, percentages, inline text wrapping via `font::measure_text_px`, atomic `InlineBlock`, **2-pass Flexbox layout** (`block.rs`, `inline.rs`, `flex.rs`), `<img>` replaced boxes (`replaced.rs`), and HTML form controls (`forms.rs`: `<input type="text|password|checkbox|radio|submit">`, `<textarea>`, `<button>`, `<select>` via `ControlKind` / `ControlBox`).
- Hit testing (`hit_test.rs`): `hit_test`, `hit_test_href`, and `hit_test_node` (returns deepest `NodeId` under pointer).

### 9. Graphics, Font Rasterizer & Display List Paint (`crates/droplet-gfx/`, `crates/droplet-font/`, `crates/droplet-paint/`)
- **Graphics & Damage (`crates/droplet-gfx/src/`):** row-major `0x00RRGGBB` `Canvas` backed by `PixelBuffer::Owned(Vec<u32>)` or `PixelBuffer::External` via `Canvas::with_external_slice` / `Canvas::with_external_slice_unfilled` (panic-unwind safe `Drop` guard), plus integer pixel-aligned `DamageRect` (`new`, `full`, `clamp`, `union`, `intersect`).
- **Typography (`crates/droplet-font/src/`):** `FontEngine` backed by `SYSTEM_FONTS` (`OnceLock<Arc<FontSet>>`), discovering Linux/Windows system TrueType fonts, parsing `sfnt`/`glyf` tables, caching rasterized alpha glyphs in a bounded `GlyphAtlas` (`MAX_ATLAS_ENTRIES = 4096`), and falling back to Cyrillic-capable secondary fonts and an `8x16` bitmap font (`GLYPH_WIDTH = 8`, `GLYPH_HEIGHT = 16`).
- **Display List, Chrome & Overlays (`crates/droplet-paint/src/`):**
  - Chrome constants: `TAB_BAR_HEIGHT = 30`, `TOOLBAR_HEIGHT = 44`, `CHROME_HEIGHT = 74`, `STATUS_BAR_HEIGHT = 0`.
  - `render_frame_damaged` clips chrome, viewport, status pill, find bar (`tools_overlay.rs`), form focus/selection/control visuals (`forms_overlay.rs`), and Box Model Inspector overlay (`BoxModelOverlay`) to `DamageRect`.

### 10. Browser Session, Storage & Cross-Platform Windowing (`crates/droplet-storage/`, `crates/droplet-shell/`, `crates/droplet-platform/`)
- **Persistent Store (`crates/droplet-storage/src/lib.rs`):** compact flat-file persistence (`Store`) bounded to `MAX_HISTORY_ENTRIES = 1000`, `MAX_BOOKMARK_ENTRIES = 256`, and `MAX_FILE_BYTES = 512 KiB`, powering history (`about:history`), the persistent cookie file (`cookies.txt`, see §3c), bookmarks (`about:bookmarks`, `Ctrl+D`), and prefix address-bar autocomplete (`Store::suggest`).
- **Browser Session (`crates/droplet-shell/src/`):** `Browser` orchestrates multi-tab `TabState` slots, **Tab Hibernation** (`MAX_WARM_TABS = 3`), concurrent `<link rel="stylesheet">` / `<img>` loading (`loader.rs`), Find-in-Page (`find.rs`), form input/caret/selection/submission (`forms.rs`), `TabJsState` + `ShellDomHost` (`js.rs`), Box Model Inspector toggle, and `take_damage()` / `set_present_backend()`.
- **Background Navigation (`crates/droplet-shell/src/nav.rs`, DROP-110):** every navigation (typed URL, link, back/forward, reload, form GET/POST, waking a hibernated tab) goes through `Browser::start_navigation`, which records a `PendingNav { nav_id, url, history: HistoryOp, restore_scroll, start }` in its tab. With a platform `Waker` installed, the page is fetched and parsed on a short-lived `droplet-nav` thread and sent back over an `mpsc` channel; `poll_navigations()` (via `PlatformApp::poll_background`) commits a result only if some tab still has that `nav_id` pending, so a newer navigation, `stop()` (`Esc`) or closing the tab cancels by dropping the entry. Back/forward stacks change only on commit (`HistoryOp::{None, Push, Back, Forward}`). Without a waker (headless CLI, tests, macOS) navigation runs inline. `Browser::new_deferred` (GUI) opens on `about:blank` and starts the initial URL once the waker exists; `about:blank` is never recorded in history.
- **Hybrid engine: WebKit views (`crates/droplet-shell/src/webview.rs`, `apps/droplet-desktop/src/bin/droplet-webview/`, DROP-170):** `start_navigation` first asks `route_to_webview`: when the platform gave an embed parent (`PlatformApp::set_embed_parent`) and `webview::wants_webview(url)` (built-in list of script-heavy sites + `~/.config/droplet/webkit-sites` + `DROPLET_WEBKIT=all|off`), the tab gets a `WebView` instead of a Droplet-rendered page. `WebView::spawn` starts `droplet-webview --embed <XID> URL` (next to the executable or `$DROPLET_WEBVIEW`): a separate process that `dlopen`s GTK 3 + WebKitGTK 4.1 + GLib, hosts one `WebKitWebView` in a `GtkPlug`, keeps cookies in SQLite under `$XDG_DATA_HOME/droplet/webkit`, and speaks a line protocol (stdin: `load/back/forward/reload/stop/resize/quit`; stdout: `ready/uri/title/load/progress`). A reader thread forwards events over `mpsc` and fires the `Waker`; `poll_webviews()` applies title/URL/history/crash. `Browser::embedded_view()` reports the active tab's window + content rect; the backend shows it. Back/forward/reload/stop go to WebKit. `TabState::clear_warm_payload` drops the `WebView`, which ends the process — hibernation frees WebKit memory too.
- **Cross-Platform Windowing & Zero-Copy Blit (`crates/droplet-platform/src/`):**
  - `lib.rs` defines `PlatformApp` (including `take_damage`, `set_present_backend`, `set_waker`, `poll_background`, `set_embed_parent` and `embedded_view`), the thread-safe `Waker` callback type, `EmbeddedView`, `NavKey`, and `CaretMove`, dispatching `run(app)` by `#[cfg(target_os = ...)]`. On Linux X11 is chosen whenever `$DISPLAY` is set (XWayland included) because only X11 can embed WebKit views; `DROPLET_BACKEND=wayland|x11` overrides.
  - **Wake-up (`linux_wake.rs`):** `WakePipe` (`UnixStream` pair as a self-pipe) + `wait_readable` (`poll(2)`): X11 waits on `XConnectionNumber` + the pipe instead of a bare `XNextEvent`; Wayland uses `wl_display_prepare_read` / `poll` / `read_events` / `dispatch_pending`. Windows posts `WM_APP + 1` to the window with `PostMessageW`.
  - **Linux Wayland (`linux_wayland.rs`):** runtime `dlopen("libwayland-client.so.0")` client implementing `xdg-shell` and `wl_shm` (`memfd_create`), rendering directly into shared memory buffers via `Canvas::with_external_slice` and issuing `wl_surface.damage_buffer` (`v4`) / `damage` (`v1-3`), reporting `"wayland-shm"` and falling back automatically to X11 if Wayland is unavailable.
  - **Linux X11 (`linux_x11.rs`):** runtime `dlopen("libX11.so.6")` + `dlopen("libXext.so.6")` with SysV `MIT-SHM` (`XShmCreateImage` / `XShmAttach` / `XShmPutImage`), rendering directly into SHM memory and blitting clamped `DamageRect` sub-rects (`"x11-shm"`), falling back cleanly to `XCreateImage` / `XPutImage` (`"x11-putimage"`). Embedding (DROP-170): after each event batch `sync_embedded` maps/moves/unmaps the foreign `EmbeddedView` window and sends the XEmbed handshake (`EMBEDDED_NOTIFY`, `WINDOW_ACTIVATE`, `FOCUS_IN`); key presses/releases go to the view with `XSendEvent` unless `is_shell_shortcut` (Droplet keeps `Ctrl+L/T/W/R/D/1–9/Tab`, `Ctrl+Shift+I`, `Alt+←/→/Home`, `F5`). A process-wide lenient X error handler keeps a vanished foreign window from killing the browser.
  - **Windows (`windows.rs`):** Win32 `CreateWindowExW` / `GetMessageW` event loop with persistent `DibSurface` (`CreateCompatibleDC` + top-down 32bpp `CreateDIBSection`), rendering directly into GDI DIB bits via `Canvas::with_external_slice` / `with_external_slice_unfilled` and `BitBlt`ing only the unioned `DamageRect` / `PAINTSTRUCT.rc_paint` (`"win32-dibsection"` with `"win32-stretchdibits"` fallback).
  - **macOS (`macos.rs`):** Cocoa/AppKit + CoreGraphics runtime FFI (`NSApplication`, `NSWindow`, flipped `NSView`, `CGContextClipToRect` dirty-rect clipping, `setNeedsDisplayInRect:`, `"cocoa"` backend reporting), verified via cross-compilation targeting `aarch64-apple-darwin`.
  - **Clipboard (`clipboard.rs`):** X11 (`xclip`/`xsel`/protocol helpers) and Win32 Unicode clipboard integration with headless-safe fallback.

### 11. Multi-Process Renderer Isolation (`crates/droplet-sandbox/`, `crates/droplet-ipc/`, `crates/droplet-shell/src/renderer.rs`, `apps/droplet-desktop/`) — frozen, cargo feature `sandbox`
- Compiled only with `--features sandbox` on `droplet-desktop` (optional `droplet-ipc` / `droplet-sandbox` deps, `tests/sandbox_renderer.rs` requires the feature). The GUI process does not use it yet (Phase 8).
- **Privilege Separation (`RendererHost` <-> `--renderer`):**
  - **Privileged Browser Process (`apps/droplet-desktop/src/renderer_host.rs`):** `RendererHost` allocates a shared pixel buffer (`ShmFrameBuffer`), spawns the `--renderer` child (`--renderer-w <W> --renderer-h <H>`) with `DROPLET_IPC_SHM_FD`, pre-fetches HTML and referenced CSS/image/script assets via `droplet-net` (`prefetch_page_payload`), sends `RenderPage`/`Scroll`/`Resize`/`Mouse*`/`JsEval`/`Shutdown` commands, reads rendered pixels straight out of shared memory, and handles child crash/EOF errors cleanly.
  - **Zero-IO Renderer Worker (`crates/droplet-shell/src/renderer.rs`, `apps/droplet-desktop/src/renderer_child.rs`):** attaches `ShmFrameBuffer::from_env_token`, warms `HeadlessRenderer` (which performs zero `droplet_net` or `std::fs` calls and resolves assets strictly from `RenderPayload.assets`), locks down the process with `apply_renderer_sandbox(SandboxProfile::StrictRenderer)`, emits `RendererToBrowser::Ready`, and paints directly into shared memory (`Canvas::with_external_slice` / `with_external_slice_unfilled`).
- **OS Sandboxing (`crates/droplet-sandbox/src/`):**
  - `SandboxProfile::{StrictRenderer, PermissiveFallback}`, `SandboxReport`, `apply_renderer_sandbox`, `apply_process_rlimits`.
  - **Linux (`linux.rs`):** `setrlimit` (`RLIMIT_CORE = 0`, `RLIMIT_NOFILE = 64`, `RLIMIT_AS = 1 GiB` clamped to hard limit), best-effort `unshare(CLONE_NEWUSER | CLONE_NEWNET)` + `/proc/self/{setgroups,uid_map,gid_map}`, `PR_SET_NO_NEW_PRIVS`, and pure-Rust `seccomp-bpf` (`AUDIT_ARCH_X86_64` / `AUDIT_ARCH_AARCH64`) blocking `socket`, `socketpair`, `connect`, `bind`, `listen`, `accept`/`accept4`, `execve`/`execveat`, `clone`/`clone3`/`fork`/`vfork`, `open`/`openat`/`openat2`, `ptrace`, `kill` (and unlisted syscalls) with `EPERM`.
  - **Windows (`windows.rs`):** Job Objects (`JOBOBJECT_EXTENDED_LIMIT_INFORMATION` + `JOBOBJECT_BASIC_UI_RESTRICTIONS`) and Restricted Tokens (`CreateRestrictedToken` with `DISABLE_MAX_PRIVILEGE`).
  - **macOS (`macos.rs`):** `setrlimit` + `sandbox_init(kSBXProfileNoNetwork)`.
- **Framed IPC & Shared Framebuffer (`crates/droplet-ipc/src/`):**
  - `ShmFrameBuffer` (`MAX_SHM_PIXELS = 64_000_000`) backed by `memfd_create` + `mmap` on Linux and `CreateFileMappingW` + `MapViewOfFile` on Windows.
  - Framed binary IPC (`IPC_MAGIC = *b"DRIP"`, `IPC_VERSION = 1`, `MAX_IPC_MESSAGE_BYTES = 32 MiB`) carrying `BrowserToRenderer` and `RendererToBrowser` messages over stdin/stdout pipes.

---

## Low-RAM Architecture (Why Droplet Stays Under Tens of MB)

1. **Zero-`HashMap` `Copy` `ComputedStyle` & Flat `Vec` DOM Attributes:** `ElementData.attributes` is `Vec<(String, String)>` (0 bytes heap allocation on attribute-less elements) and `ComputedStyle` derives `Copy` with no per-node `HashMap`.
2. **Contiguous Arena DOM (`Vec<Node>`) + Iterative Traversals:** Eliminates thousands of individual `Rc<RefCell<...>>` node allocations on DOM layout/style trees and avoids stack overflows on deeply nested HTML.
3. **Automatic Tab Hibernation (`MAX_WARM_TABS = 3`):** Background tabs past the warm LRU limit shed DOM, layout, display list, and decoded image trees while retaining URL, title, and navigation history.
4. **Bounded Caches & Media Guards:** `MAX_ATLAS_ENTRIES = 4096` glyphs, `SYSTEM_FONTS` shared via `OnceLock<Arc<FontSet>>`, `MAX_IMAGE_PIXELS = 64_000_000`, `MAX_RESPONSE_BYTES = 16 MiB`, `DNS_CACHE_MAX_ENTRIES = 256`, `POOL_MAX_TOTAL = 32`, `MAX_HISTORY_ENTRIES = 1000`, `MAX_BOOKMARK_ENTRIES = 256`, cookies `50`/domain + `3000` total + `4 KiB` each, `MAX_DOM_LISTENERS = 1024`, and `TrackerFilter` blocking before DNS/TCP.
5. **Direct External-Slice Rendering & Partial Damage (`DamageRect`):** Wayland `wl_shm`, X11 `MIT-SHM`, and Win32 `DIBSection` render directly into OS/shared pixel memory and present only damaged sub-rectangles, while `DirtyFlags` avoids full relayout on paint-only JS/DOM changes and blocking event loops keep idle CPU near `~0%`.

### Measurement & Regression Harness (Phase 0)
- **`droplet --bench [--runs N] URL`** (`crates/droplet-shell/src/bench.rs`, `BenchReport`): fetch + subresources once, then N runs of `html::parse` → `css::parse` → `style_tree_with_context` → `layout_tree` → `build_display_list` → `render_frame`; prints the median per stage, tree sizes and peak RSS (`VmHWM`, Linux).
- **Corpus goldens** (`tests/corpus/*.html`, `apps/droplet-desktop/tests/corpus.rs`): `--dump-layout` under `DROPLET_FONTS=builtin` compared with `tests/corpus/expected/*.layout`; `manifest.txt` records the human-reviewed `ok`/`wrong` status per page.
- `--dump-layout` and `--screenshot` both go through `Browser` (one pipeline, same loader).

