# Droplet Engine Architecture

**Droplet** is a zero-bloat graphical web browser engine written from scratch in pure Rust (`edition = "2021"`).  
`Cargo.toml` keeps `[dependencies]` **completely empty**—zero external crates, zero async runtimes, and zero build-time C/C++ toolchains. OS integration relies strictly on Rust `std` plus runtime dynamic library loading (`dlopen` on Linux for `libX11.so.6` and `libssl.so.4`/`.3`/`.so`, Win32 API / `StretchDIBits` on Windows, and an architectural `macos.rs` stub).

---

## End-to-End Rendering Pipeline

```text
  Input URL / CLI / Click / Tab Switch
           │
           ▼
   [1] crates/droplet-url/src/lib.rs          Url::parse / Url::resolve (http://, https://, file://, about:*)
           │
           ▼
   [2] crates/droplet-net/src/adblock.rs      TrackerFilter::should_block (zero-alloc domain/suffix check before DNS/TCP)
     + crates/droplet-net/src/lib.rs          DNS TTL cache -> HTTP/1.1 + Keep-Alive pool -> OpenSSL TLS (Linux)
                           fetch_many (4-worker std::thread pool) / fetch_uri_bytes (data:, file:, http(s):)
           │
           ▼
   [3] crates/droplet-dom/src/html.rs         html::parse (iterative UTF-8-safe stack parser)
           │
           ▼
   [4] crates/droplet-dom/src/arena.rs          Document / NodeId arena (Vec<Node>, Vec<(String, String)> attributes)
                           collect_style_text() & collect_stylesheet_links()
           │
           ▼
   [5] crates/droplet-image/src/lib.rs        image::decode -> DecodedImage (pure-Rust PNG/BMP/PPM/SVG, 64M-px guard)
           │
           ▼
   [6] crates/droplet-style/src/css.rs          css::parse / parse_inline_style (Flexbox, %/em/rem/vh/vw, :hover/:active)
           │
           ▼
   [7] crates/droplet-style/src/cascade.rs        style_tree_with_context (Copy ComputedStyle, zero HashMap, StyleContext)
           │
           ▼
   [8] crates/droplet-layout/src/lib.rs       layout_tree (Block, Inline, InlineBlock, 2-pass Flexbox, <img> replaced)
                           hit_test / hit_test_href / hit_test_node
           │
           ▼
   [9] crates/droplet-paint/src/lib.rs        build_display_list -> DisplayList -> render_frame -> Canvas
     + crates/droplet-font/src/lib.rs         FontEngine (Linux/Windows TTF discovery, pure-Rust sfnt/glyf AA,
                           bounded GlyphAtlas max 4096 entries, 8x16 bitmap fallback)
           │
           ├──────────────────────────────────────────┐
           ▼                                          ▼
  [10a] crates/droplet-platform/src/                        [10b] Headless CLI (apps/droplet-desktop/src/main.rs)
        mod.rs -> linux_x11.rs / windows.rs        --dump-dom / --dump-layout / --screenshot
        (window.rs compatibility alias)
```

The orchestration layer (`crates/droplet-shell/src/lib.rs` -> `Browser`) manages multi-tab `TabState` slots, automatic **Tab Hibernation** past `MAX_WARM_TABS = 3`, per-tab navigation history (`back_stack` / `forward_stack`), concurrent external CSS and `<img>` fetching, scroll offsets, URL editing state, cached `LayoutBox` / `DisplayList`, and dirty repaint flags (`needs_repaint`).

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

### 3. Transport, Pooling, TLS & Built-in Pages (`crates/droplet-net/src/lib.rs`)
- `Response`: `{ url: Url, status: u16, content_type: String, body: String }`
- `NetError`: `{ message: String }`
- **Public API:** `fetch`, `fetch_with_filter`, `fetch_many`, `fetch_many_with_filter`, `fetch_bytes`, `fetch_uri_bytes` (handles `data:` URIs + resolved `file:`/`http:`/`https:` URLs), plus pure testable parsers `parse_http_response` and `decode_chunked`.
- **Caching & Pooling (`NET_STATE`):**
  - **DNS cache:** `(host, port) -> DnsEntry` with `DNS_TTL = 60s` and `DNS_CACHE_MAX_ENTRIES = 256`.
  - **HTTP/1.1 Keep-Alive connection pool:** keyed by `(SchemeKey, host, port)` with `POOL_IDLE_TTL = 30s`, `POOL_MAX_PER_ORIGIN = 4`, and `POOL_MAX_TOTAL = 32`. Reads framed `Content-Length` and incremental `Transfer-Encoding: chunked` bodies without waiting for EOF, and transparently retries once on stale pooled sockets.
- **TLS (`https://`):**
  - On **Linux**, `tls::TlsStream` dynamically loads `libssl.so.4`, `libssl.so.3`, or `libssl.so` at runtime via `dlopen`, configures system CA verification (`SSL_CTX_set_default_verify_paths` + `SSL_VERIFY_PEER`), sets SNI (`SSL_set_tlsext_host_name`), and verifies the peer hostname (`SSL_set1_host` + `SSL_get_verify_result`).
  - On **non-Linux targets** (Windows/macOS), `tls::TlsStream` returns a descriptive `NetError` pending native Schannel/system TLS implementation (`DROP-011`).
- **Concurrency & Limits:** `fetch_many` uses a bounded `std::thread::scope` pool (`MAX_CONCURRENT_FETCHES = 4`); enforces `HTTP_TIMEOUT = 5s`, `MAX_REDIRECTS = 5`, `MAX_HEADER_BYTES = 64 KiB`, and `MAX_RESPONSE_BYTES = 16 MiB`.

### 4. Pure-Rust Image Decoder (`crates/droplet-image/src/lib.rs`)
- `DecodedImage`: `{ pub width: u32, pub height: u32, pub rgba: Vec<u8> }` (row-major RGBA8).
- `ImageError`: `{ pub message: String }`.
- `decode(&[u8]) -> Result<DecodedImage, ImageError>` sniffs magic bytes and decodes in pure Rust (zero external crates):
  - **PNG:** full chunk reader, DEFLATE/zlib inflate, all 5 scanline predictors (`None`/`Sub`/`Up`/`Average`/`Paeth`), Adam7 interlacing, grayscale/RGB/RGBA/indexed `PLTE` + `tRNS`.
  - **BMP:** uncompressed/RLE headers into RGBA8.
  - **PPM:** ASCII `P3` and binary `P6`.
  - **Static SVG:** XML preamble + `<svg>` (`viewBox`, `<rect>`, `<circle>`, `<line>`, `<polygon>`, `<path>` fill/stroke rasterization).
  - **JPEG:** cleanly rejected (`unsupported image format: JPEG`) pending `DROP-011` baseline JPEG decoder.
- **Dimension Bomb Guard:** `validate_dimensions` enforces `width * height <= MAX_IMAGE_PIXELS` (`64_000_000`) and checked byte-size math before allocating `Vec<u8>`.

### 5. Arena DOM (`crates/droplet-dom/src/arena.rs`) & HTML Parser (`crates/droplet-dom/src/html.rs`)
- `NodeId = usize`
- `ElementData`: `{ pub tag_name: String, pub attributes: Vec<(String, String)> }` with `id()`, `classes()`, `has_class()`, `attr()`. Storing attributes in a flat `Vec` instead of `HashMap` incurs **zero heap allocation** on attribute-less elements (`<p>`, `<span>`, `<div>`, `<li>`, etc.).
- `NodeKind`: `Document | Element(ElementData) | Text(String)`
- `Node`: `{ id: NodeId, parent: Option<NodeId>, children: Vec<NodeId>, kind: NodeKind }`
- `Document`: `{ nodes: Vec<Node> }` where `nodes[0]` is always `NodeKind::Document`.
- Iterative `Vec`-stack traversals (zero recursion): `text_content`, `collect_style_text`, `collect_stylesheet_links` (`<link rel="stylesheet" href="...">` in document order), `find_title`, `find_first_tag`, `dump_tree`.
- `html::parse(&str) -> Document`: iterative UTF-8-safe HTML5-style stack parser handling `<!DOCTYPE>`, comments, void tags, self-closing tags, raw text (`<style>`, `<script>`), HTML entities, and implicit auto-closing rules.

### 6. CSS Parser (`crates/droplet-style/src/css.rs`) & Style Tree (`crates/droplet-style/src/cascade.rs`)
- **CSS Parser (`crates/droplet-style/src/css.rs`):**
  - `Color`: `{ r: u8, g: u8, b: u8, a: u8 }` (`to_u32_xrgb`, `from_name_or_hex`).
  - `Unit`: `Px | Percent | Em | Rem | Vh | Vw`.
  - `Value`: `Keyword(String) | Length(f32, Unit) | Color(Color) | Number(f32)`.
  - `CompoundSelector`: `{ tag_name, id, classes, universal, hover, active, unsupported_pseudo }` — `:hover` and `:active` each count in the class specificity bucket `(0, 1, 0)`.
  - `ComplexSelector` (`Descendant` / `Child` combinators), `Rule`, `Stylesheet`.
- **Style Tree (`crates/droplet-style/src/cascade.rs`):**
  - `Display`: `Block | Inline | InlineBlock | Flex | None`.
  - Flexbox enums: `FlexDirection`, `FlexWrap`, `JustifyContent`, `AlignItems`, `FlexBasis`.
  - `ComputedStyle`: derives `Copy` and contains **zero `HashMap`s** or heap allocations—only typed scalar/enum/edge fields (`display`, `color`, `background_color`, `border_color`, `margin`, `padding`, `border`, `border_radius`, `width`/`width_pct`, `max_width`/`max_width_pct`, `height`/`height_pct`, `font_size`, `bold`, `monospace`, `text_underline`, `margin_left_auto`, `margin_right_auto`, `preformatted`, and Flexbox properties).
  - `StyleContext`: `{ viewport_width: f32, viewport_height: f32, hovered_node: Option<NodeId>, active_node: Option<NodeId> }` enables accurate `vh`/`vw` resolution and ancestor-propagated `:hover`/`:active` matching.
  - `stylesheet_uses_interaction(&Stylesheet) -> bool`: fast pre-check allowing `Browser` to skip restyle/relayout on mouse motion when a page has no `:hover` or `:active` rules.

### 7. Box Layout Engine (`crates/droplet-layout/src/lib.rs`)
- `Rect` / `Dimensions` (`content`, `padding`, `border`, `margin`).
- `BoxType`: `Block | Inline | InlineBlock | Flex | AnonymousBlock | Text { lines, line_height, font_size, font_scale, monospace } | Image { image: Option<Arc<DecodedImage>>, alt: String }`.
- `LayoutBox`: `{ node_id: Option<NodeId>, dimensions, box_type, style, href, children }`.
- `layout_tree(doc, styled_root, viewport_width, viewport_height, images)`:
  - Resolves block widths, auto margins, percentages, and vertical flow.
  - Wraps inline text and atomic `InlineBlock` boxes across line boxes using `font::measure_text_px`.
  - Runs a **2-pass Flexbox layout** (`FlexDirection::{Row, Column, RowReverse, ColumnReverse}`, `FlexWrap`, `JustifyContent`, `AlignItems`, `gap`, `flex_grow`, `flex_shrink` clamped non-negative, `flex_basis`).
  - Lays out `<img>` as replaced boxes preserving intrinsic aspect ratio when one dimension is specified, or rendering an `alt`-text placeholder box on decode/load failure.
- Hit testing: `hit_test`, `hit_test_href`, and `hit_test_node` (returns deepest `NodeId` under pointer for `:hover`/`:active`).

### 8. Font Rasterizer (`crates/droplet-font/src/lib.rs`) & Display List Paint (`crates/droplet-paint/src/lib.rs`)
- **Typography (`crates/droplet-font/src/lib.rs`):**
  - `FontEngine` backed by process-wide `SYSTEM_FONTS` (`OnceLock<Arc<FontSet>>`).
  - Discovers system TrueType fonts on **Linux** (`/usr/share/fonts/truetype/ubuntu/...`, `dejavu/...`, `liberation/...`, `noto/...`) and **Windows** (`C:\Windows\Fonts\segoeui.ttf`, `arial.ttf`, `consola.ttf`, `cour.ttf`).
  - Pure-Rust `sfnt`/`glyf` table parser (`head`, `maxp`, `cmap` format 4/12, `hhea`, `hmtx`, `loca`, `glyf` simple + compound glyphs up to `MAX_COMPOUND_DEPTH = 8`) and anti-aliased scanline rasterizer.
  - Bounded `GlyphAtlas` cache (`HashMap<GlyphCacheKey, RasterGlyph>`) capped at `MAX_ATLAS_ENTRIES = 4096` entries.
  - Automatic per-glyph fallback to secondary system fonts (e.g. `DejaVuSans.ttf` for Cyrillic) and the built-in `8x16` monochrome bitmap font (`GLYPH_WIDTH = 8`, `GLYPH_HEIGHT = 16`).
- **Display List & Chrome (`crates/droplet-paint/src/lib.rs`):**
  - Minimalist browser chrome constants: `TAB_BAR_HEIGHT = 30`, `TOOLBAR_HEIGHT = 44`, `CHROME_HEIGHT = 74`, `STATUS_BAR_HEIGHT = 0`.
  - `DisplayCommand`: `SolidRect { rect, color, radius }`, `Text { x, y, text, color, font_size, scale, bold, monospace, underline, width }`, `Image { rect, image }`.
  - `render_frame` performs O(1) vertical viewport culling over `DisplayList`, draws SDF anti-aliased rounded rectangles (`fill_rounded_rect_clipped`, `draw_rounded_outline_clipped`), alpha-blended TrueType glyphs, RGBA images (`draw_image_clipped`), a floating scrollbar thumb, the top tab strip + toolbar + timing pill, and a **floating bottom-left hover URL pill** (`draw_floating_status_pill`) without dedicating a permanent bottom bar.

### 9. Browser Session (`crates/droplet-shell/src/lib.rs`) & Cross-Platform Windowing (`crates/droplet-platform/src/`)
- `Browser` owns `Vec<TabState>`, `active_tab`, `MAX_WARM_TABS = 3`, `FontEngine` / viewport size, pointer state `(mouse_x, mouse_y, mouse_down)`, and `needs_repaint`.
  - **Multi-Tab & Tab Hibernation:** Each `TabState` holds its URL, title, `back_stack`, `forward_stack`, `scroll_y`, and optional warm render payload (`Document`, `Stylesheet`, `LayoutBox`, `DisplayList`, decoded image map). When active + recent warm tabs exceed `MAX_WARM_TABS = 3`, the oldest inactive background tabs are hibernated (dropping parsed DOM, layout tree, display list, and decoded images while retaining URL/title/history) and transparently reloaded when switched back to.
  - **Concurrent Subresource Loading:** Collects `<link rel="stylesheet">` via `doc.collect_stylesheet_links()` and `<img src="...">` URIs, resolving and fetching them concurrently via `net::fetch_many` / `net::fetch_uri_bytes` before running `style_tree_with_context` + `layout_tree` + `build_display_list`.
  - **Zero-Waste Interaction:** Mouse motion updates `hovered_node` via `layout::hit_test_node` and only triggers restyle/relayout/repaint if `hovered_node` changed **and** `stylesheet_uses_interaction(&stylesheet)` is true.
- **Cross-Platform Windowing (`crates/droplet-platform/src/`):**
  - `crates/droplet-platform/src/lib.rs` defines the `PlatformApp`/`NavKey` contract and dispatches `run(app)` by `#[cfg(target_os = ...)]`.
  - `crates/droplet-platform/src/linux_x11.rs`: runtime `dlopen("libX11.so.6")`, blocking `XNextEvent` loop, batched expose/motion/configure handling, `XCreateImage` + `XPutImage` blit.
  - `crates/droplet-platform/src/linux_wayland.rs`: runtime `dlopen("libwayland-client.so.0")`, hand-written xdg-shell + `wl_shm` client, automatic fallback to X11.
  - `crates/droplet-platform/src/windows.rs`: Win32 `CreateWindowExW` / `GetMessageW` event loop + `StretchDIBits` XRGB framebuffer presentation.
  - `crates/droplet-platform/src/macos.rs`: native Cocoa/`CGContext` implementation when built on macOS with the Apple SDK.

---

## Low-RAM Architecture (Why Droplet Stays Under Tens of MB)

Droplet is engineered so a running session with multiple tabs stays in the **low tens of megabytes** rather than hundreds of megabytes per tab.

### Implemented Invariants (Live Today)
1. **Zero-`HashMap` `Copy` `ComputedStyle` & Flat `Vec` DOM Attributes:**
   - `ElementData.attributes` is `Vec<(String, String)>` instead of `HashMap<String, String>`. Elements without attributes (`<p>`, `<span>`, `<div>`, `<li>`, `<td>`) allocate **0 bytes** on the heap for attributes, saving ~48–96 bytes + bucket overhead per element.
   - `ComputedStyle` derives `Copy` and stores typed fields directly—zero per-node `HashMap<String, Value>` allocations across the entire `StyledNode` and `LayoutBox` trees.
2. **Contiguous Arena DOM (`Vec<Node>`) + Iterative Traversals:**
   - Every HTML node sits sequentially in `Document.nodes: Vec<Node>` indexed by `NodeId = usize`, eliminating thousands of individual `Rc<RefCell<...>>` heap allocations and pointer-chasing cache misses.
3. **Automatic Tab Hibernation (`MAX_WARM_TABS = 3`):**
   - Opening 10+ tabs does **not** keep 10+ full DOM, Layout, DisplayList, and decoded RGBA image trees resident in RAM. Once warm tabs exceed `MAX_WARM_TABS = 3`, the least-recently-used background tabs shed their heavy rendering state and keep only URL, title, and navigation history until selected again.
4. **Bounded `GlyphAtlas` (`MAX_ATLAS_ENTRIES = 4096`) & Shared `SYSTEM_FONTS`:**
   - Font bytes (`TrueTypeFont`) are parsed once into `OnceLock<Arc<FontSet>>` and shared across tabs. The rasterized alpha glyph cache is hard-capped at `4096` entries so long browsing sessions cannot leak unbounded bitmap memory.
5. **Memory Guards on Network & Media:**
   - `image::decode` enforces `MAX_IMAGE_PIXELS = 64_000_000` **before** allocating RGBA pixel buffers, neutralizing decompression/dimension bombs.
   - `net.rs` caps response bodies (`MAX_RESPONSE_BYTES = 16 MiB`), headers (`64 KiB`), DNS entries (`256`), and pooled sockets (`32`), while `TrackerFilter` blocks ad/telemetry requests before DNS/TCP or body allocation even happens.
6. **Single Reused Framebuffer + O(1) DisplayList Culling + Event-Driven Sleep:**
   - `Canvas.pixels` (`Vec<u32>`) is resized in place on window configure events; scrolling walks the cached flat `DisplayList` with O(1) Y-bounds culling; `stylesheet_uses_interaction` avoids restyle/relayout on mouse moves over static pages; blocking `XNextEvent` / `GetMessageW` keeps idle CPU at `~0%`.

### Planned Low-RAM Optimizations (Next Phases)
1. **String Interning / Arena Text Slice Pool:** Intern repeated HTML tag names, attribute keys, class tokens, and short text nodes into a per-document arena buffer instead of individual `String` heap allocations.
2. **LRU Decoded Image Byte Budget & Downscaling:** Keep decoded RGBA images bounded by a strict megabyte budget across active tabs, evicting off-screen images or storing display-resolution mipmaps rather than keeping oversized source bitmaps resident.
3. **Linear Arena `LayoutBox` Tree:** Flatten `LayoutBox.children: Vec<LayoutBox>` into an indexed `Vec<LayoutBox>` arena (`LayoutId(usize)`) to eliminate per-box child `Vec` heap allocations.
4. **Zero-Copy Surface Blitting (`DROP-009`):** Present directly via X11 `MIT-SHM` / Win32 `DIBSection` / GPU texture upload so the compositor avoids duplicate pixel-buffer copies.
