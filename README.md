# Droplet Browser

**Droplet** is a fast, zero-bloat, multi-tab graphical web browser engine written from scratch in Rust (`edition = "2021"`) for **Linux (Wayland/X11)**, **Windows (Win32)**, **macOS (Cocoa)**, and headless CLI.  
It is a Cargo workspace of focused engine crates with **zero external crate dependencies**—pure Rust `std` plus runtime OS dynamic loading (`libwayland-client.so.0` / `libX11.so.6` / `libXext.so.6` / `libssl.so` on Linux, Win32 GDI `DIBSection` + Schannel on Windows, and Objective-C/CoreGraphics FFI on macOS).

---

> **Status (Nov 2026 re-baseline):** the core pipeline works and is fast, but much of CSS layout is
> still missing — only 4 of 12 pages in `tests/corpus` render correctly (no tables, `position`,
> `float`, `@media`, …). See [ROADMAP.md](ROADMAP.md) for the honest scoreboard and phases.

## Features

- **HTTPS, Keep-Alive & Built-In Tracker Blocker:**
  - `https://` on **Linux** via runtime `dlopen` OpenSSL (`libssl.so.4` / `libssl.so.3` / `libssl.so`) and on **Windows** via native Win32 **Schannel** (`secur32.dll` / `crypt32.dll`) with system CA verification and SNI, alongside `http://`, `file://`, `data:`, and `about:*`.
  - HTTP/1.1 `Keep-Alive` connection pool, in-memory DNS TTL cache, and bounded 4-thread concurrent loader for external `<link rel="stylesheet">` stylesheets and images.
  - Zero-allocation domain/suffix **Tracker Filter** (`crates/droplet-net/src/adblock.rs`) blocking ad/telemetry hosts before DNS or TCP connect.
- **Modern Layout, Forms & Interactive CSS:**
  - Two-pass **Flexbox** (`display: flex`, `flex-direction`, `flex-wrap`, `justify-content`, `align-items`, `gap`, `flex-grow`/`flex-shrink`/`flex-basis`) and `display: inline-block`.
  - Relative units (`%`, `em`, `rem`, `vh`, `vw`), rounded rectangles (`border-radius`), `:hover` / `:active` pseudo-classes, and HTML form controls (`<input>`, `<textarea>`, `<button>`, `<select>`) with `Tab`/`Shift+Tab` focus, caret editing, text selection, and clipboard integration.
- **Experimental JavaScript VM (off by default, enable with `--js`):**
  - Covers only an ES subset (no `new`, classes or template literals yet), so it is disabled until the Phase 7 decision. Pure-Rust bytecode VM (`crates/droplet-js`) with `DomHost` bindings (`document.getElementById`, `querySelector`, `querySelectorAll`, `addEventListener`, `textContent`, `innerHTML`, `classList`, `style`, timers), event bubbling (`preventDefault` / `stopPropagation`), and `DirtyFlags` incremental style/layout/display-list updates.
- **Anti-Aliased TrueType Fonts & Image Decoding:**
  - System TrueType font discovery on Linux (`/usr/share/fonts/...`) and Windows (`C:\Windows\Fonts\...`) with a pure-Rust `sfnt`/`glyf` AA scanline rasterizer, bounded `GlyphAtlas` (`4096` entries), and `8x16` ASCII + Cyrillic bitmap fallback.
  - Pure-Rust **PNG, baseline JPEG, BMP, PPM (`P3`/`P6`), and static SVG** image decoder (`crates/droplet-image`) with a 64M-pixel bomb guard and `<img>` replaced layout boxes (`alt`-text fallback on error).
- **Daily-Driver UI, Persistence & Low-RAM Design:**
  - Minimalist `30px` tab strip + `44px` toolbar + floating hover URL pill (`STATUS_BAR_HEIGHT = 0`), persistent bookmarks & history (`crates/droplet-storage`, `about:history`, `about:bookmarks`) with URL autocomplete, Find-in-Page (`Ctrl+F`), and Box Model Inspector (`Ctrl+Shift+I`).
  - Automatic **Tab Hibernation** beyond 3 warm tabs (`MAX_WARM_TABS = 3`), Arena DOM (`Vec<Node>`), flat `Vec` attributes, and `Copy` `ComputedStyle` (zero `HashMap` per element).
- **Zero-Copy Windowing & Multi-Process Sandbox (`crates/droplet-platform/`, `crates/droplet-sandbox/`, `crates/droplet-ipc/`):**
  - **Linux:** **X11** `MIT-SHM` / `XPutImage` (default whenever `$DISPLAY` is set, XWayland included), native **Wayland** (`wl_shm` + `xdg-shell`) without X or with `DROPLET_BACKEND=wayland`.
- **Hybrid engine (DROP-170):** documents render with Droplet's own engine; script-heavy sites (YouTube, Google, Telegram, VK, GitHub…; extend in `~/.config/droplet/webkit-sites`, `*` = all; `DROPLET_WEBKIT=all|off`) open in a tab backed by the out-of-process `droplet-webview` helper (system WebKitGTK 4.1 via `dlopen`, embedded with XEmbed). Hibernating such a tab ends its WebKit process. Video needs `gstreamer1.0-plugins-bad` + `gstreamer1.0-libav`.
  - **Windows:** Win32 `CreateWindowExW` with persistent `DIBSection` direct rendering + `DamageRect` `BitBlt` (`StretchDIBits` fallback).
  - **macOS:** Cocoa/AppKit `NSView` + `CGContextClipToRect` dirty-rect presentation (verified via cross-compilation for `aarch64-apple-darwin`).
  - **Experimental sandboxed renderer worker (`cargo build --features sandbox`; `--sandbox-screenshot` / `--renderer`; not yet used by the GUI):** privileged `RendererHost` pre-fetches resources over `droplet-net` and communicates with an isolated `HeadlessRenderer` worker over framed `DRIP` IPC + `memfd`/Win32 shared memory (`droplet-ipc`) under `droplet-sandbox` (Linux `seccomp-bpf` + best-effort user/net namespaces + `rlimit`; Windows Job Objects + Restricted Token; macOS `sandbox_init`).

---

## Repository Layout

```text
apps/droplet-desktop/     CLI entry point, headless modes & multi-process RendererHost / --renderer
crates/droplet-url/       URL parser and resolver
crates/droplet-net/       HTTP/HTTPS (OpenSSL/Schannel), DNS cache, keep-alive pool, cookie jar, tracker filter, about:* pages
crates/droplet-dom/       Arena DOM, iterative HTML parser, mutation & CSS selector queries
crates/droplet-js/        Pure-Rust ES-subset lexer, compiler, stack bytecode VM & DomHost bridge
crates/droplet-ipc/       Shared-memory framebuffer (memfd/Win32) and framed DRIP IPC protocol
crates/droplet-sandbox/   OS process isolation (Linux seccomp-bpf/namespaces, Win32 Job/Token, macOS sandbox)
crates/droplet-style/     CSS parser, cascade, Copy ComputedStyle
crates/droplet-layout/    Block/inline/flex/form layout, hit testing
crates/droplet-gfx/       Canvas (owned & external-slice buffers), colors, DamageRect geometry
crates/droplet-font/      TrueType discovery, sfnt/glyf rasterizer, glyph atlas
crates/droplet-inflate/   DEFLATE / zlib / gzip decompression (PNG IDAT, HTTP Content-Encoding)
crates/droplet-image/     PNG, baseline JPEG, BMP, PPM, and static SVG decoders
crates/droplet-paint/     Flat display list, chrome, find/inspector/form overlays
crates/droplet-platform/  Wayland, X11, Win32, macOS windowing & clipboard backends
crates/droplet-storage/   Persistent binary/flat-file history, bookmarks, cookie file & URL autocomplete store
crates/droplet-shell/     Multi-tab browser session, loader, forms, find, JS DOM host & zero-IO HeadlessRenderer
apps/droplet-desktop/src/bin/droplet-webview/  WebKitGTK host process (dlopen, XEmbed)
```

`Cargo.toml` is a virtual workspace covering all crates above plus `apps/droplet-desktop` (binaries `droplet` and `droplet-webview`).

---

## Quick Start

### Prerequisites
- Rust toolchain (`/home/mrevds/.cargo/bin/cargo` / `rustc`)
- **Linux GUI:** X11 (`libX11.so.6`, optional `libXext.so.6` for `MIT-SHM`) or Wayland (`libwayland-client.so.0`); `libssl.so.4`/`.3`/`.so` for `https://`
- **Optional, for YouTube/mail/messengers:** WebKitGTK 4.1 (`libwebkit2gtk-4.1.so.0`, GTK 3); for video also `gstreamer1.0-plugins-bad gstreamer1.0-libav`
- **Windows GUI:** Win32 (`user32.dll`, `gdi32.dll`, `secur32.dll`, `crypt32.dll`)
- Headless CLI modes (`--dump-dom`, `--dump-layout`, `--screenshot`) run without any display server.

### Build & Test

```bash
/home/mrevds/.cargo/bin/cargo test --workspace
/home/mrevds/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings
/home/mrevds/.cargo/bin/cargo build --release
```

---

## Running Droplet

```bash
# Open the built-in welcome page (about:droplet) in a GUI window
/home/mrevds/.cargo/bin/cargo run -- about:droplet

# Open a local HTML file or HTTP/HTTPS page
/home/mrevds/.cargo/bin/cargo run -- /path/to/index.html
/home/mrevds/.cargo/bin/cargo run -- https://example.com/
```

### Headless CLI Flags

```bash
# Parse HTML and print the Arena DOM tree
/home/mrevds/.cargo/bin/cargo run -- --dump-dom about:test

# Compute styles and print the layout box tree
/home/mrevds/.cargo/bin/cargo run -- --dump-layout about:test

# Render a full frame (tabs + toolbar + page) to a binary PPM image
/home/mrevds/.cargo/bin/cargo run -- --screenshot /tmp/out.ppm about:droplet

# Per-stage benchmark: median ms for html/css/style/layout/display-list/raster + peak RSS
/home/mrevds/.cargo/bin/cargo run --release -- --bench --runs 20 about:droplet

# Enable the experimental JavaScript VM for any mode (off by default)
/home/mrevds/.cargo/bin/cargo run -- --js --screenshot /tmp/out.ppm about:test

# Experimental: render inside the sandboxed --renderer child (needs --features sandbox)
/home/mrevds/.cargo/bin/cargo run --features sandbox -- --sandbox-screenshot /tmp/sandbox.ppm about:droplet
```

`DROPLET_FONTS=builtin` forces the built-in 8x16 font (machine-independent metrics; used by the
corpus layout goldens in [tests/corpus](tests/corpus/README.md)).

### Built-in `about:*` Pages
- `about:droplet` — Welcome page & navigation hub
- `about:roadmap` — Phased development plan
- `about:architecture` — Rendering pipeline & Arena DOM overview
- `about:test` — Box Model, Flexbox, Cyrillic/ASCII typography, links, lists & `<pre><code>` visual test page
- `about:history` — Browsing history view & clear/remove actions
- `about:bookmarks` — Saved bookmarks view & remove action
- `about:blank` — Empty page

---

## Controls (GUI Window)

| Input | Action |
| --- | --- |
| **Left Click** | Activate `<a href>` links, interact with form controls, switch/close tabs, click `+` new tab, or toolbar buttons |
| **Mouse Wheel** | Scroll page up / down |
| **Mouse Buttons 8 / 9** | History Back / Forward |
| `Ctrl+T` / `Ctrl+W` | Open a new tab / Close active tab |
| `Ctrl+Tab` / `Ctrl+Shift+Tab` / `Ctrl+1..9` | Cycle next/prev tab or jump directly to tab 1–9 |
| `Ctrl+L` / `Enter` / `Esc` | Focus address bar / Navigate (or accept autocomplete) / Stop a loading page, else unfocus or dismiss bar |
| `Ctrl+D` | Bookmark / unbookmark current page |
| `Ctrl+F` (`Enter` / `Shift+Enter`) | Open Find-in-Page (`Next` / `Previous` match) |
| `Ctrl+Shift+I` | Toggle Box Model Inspector overlay |
| `Ctrl+A` / `Ctrl+C` / `Ctrl+X` / `Ctrl+V` | Select all / Copy / Cut / Paste |
| `Ctrl+R` or `F5` | Reload current page |
| `Alt+Left` / `Alt+Right` / `Alt+Home` | History Back / Forward / Go Home (`about:droplet`) |
| `Up` / `Down` / `PageUp` / `PageDown` / `Home` / `End` / `Space` | Scroll page |

---

## Documentation

- [ARCHITECTURE.md](ARCHITECTURE.md) — End-to-end rendering pipeline, module map, cross-platform layer, and Low-RAM invariants.
- [ROADMAP.md](ROADMAP.md) — Phased development roadmap & Cross-Platform Windowing Strategy (in Russian with standard technical terms in English).
- [CLAUDE.md](CLAUDE.md) — Engineering standards, Linux + Windows rules, and autonomous workflow instructions for AI agents.
- [.scratch/SPEC.md](.scratch/SPEC.md) — Frontier issue tracker (`DROP-001` .. `DROP-011`).
# droplet-browser
