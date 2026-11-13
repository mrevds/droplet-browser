# Droplet Browser

**Droplet** is a fast, zero-bloat, multi-tab graphical web browser engine written from scratch in Rust (`edition = "2021"`) for **Linux (X11/Wayland)**, **Windows (Win32)**, and headless CLI.  
It is a Cargo workspace of small engine crates with **zero external crate dependencies**—pure Rust `std` plus runtime OS dynamic loading (`libX11.so.6` / `libwayland-client.so.0` / `libssl.so` on Linux, Win32 `StretchDIBits` + Schannel on Windows, and a native Cocoa slot for macOS).

---

## Features

- **HTTPS, Keep-Alive & Built-In Tracker Blocker:**
  - `https://` on Linux via runtime `dlopen` OpenSSL (`libssl.so.4` / `libssl.so.3` / `libssl.so`) with system CA verification and SNI, alongside `http://`, `file://`, `data:`, and `about:*`.
  - HTTP/1.1 `Keep-Alive` connection pool, in-memory DNS TTL cache, and bounded 4-thread concurrent loader for external `<link rel="stylesheet">` stylesheets and images.
  - Zero-allocation domain/suffix **Tracker Filter** (`crates/droplet-net/src/adblock.rs`) blocking ad/telemetry hosts before DNS or TCP connect.
- **Modern Layout & Interactive CSS:**
  - Two-pass **Flexbox** (`display: flex`, `flex-direction`, `flex-wrap`, `justify-content`, `align-items`, `gap`, `flex-grow`/`flex-shrink`/`flex-basis`) and `display: inline-block`.
  - Relative units (`%`, `em`, `rem`, `vh`, `vw`), rounded rectangles (`border-radius`), and `:hover` / `:active` pseudo-classes (with zero mouse-move overhead when a page uses no interactive rules).
- **Anti-Aliased TrueType Fonts & Image Decoding:**
  - Automatic system TrueType font discovery on Linux (`/usr/share/fonts/...`) and Windows (`C:\Windows\Fonts\...`) with a pure-Rust `sfnt`/`glyf` AA scanline rasterizer, bounded `GlyphAtlas` cache (`4096` entries), and `8x16` ASCII + Cyrillic bitmap fallback.
  - Pure-Rust **PNG, baseline JPEG, BMP, PPM (`P3`/`P6`), and static SVG** image decoder (`crates/droplet-image/src/lib.rs`) with a 64M-pixel bomb guard and `<img>` replaced layout boxes (`alt`-text fallback on error).
- **Minimalist Multi-Tab UI & Low-RAM Design:**
  - Clean `30px` tab strip + `44px` toolbar + floating bottom-left hover URL pill (`STATUS_BAR_HEIGHT = 0`).
  - Automatic **Tab Hibernation** beyond 3 warm tabs (`MAX_WARM_TABS = 3`), Arena DOM (`Vec<Node>`), flat `Vec` DOM attributes, and `Copy` `ComputedStyle` (zero `HashMap` per element).
- **Cross-Platform Windowing (`crates/droplet-platform/src/`):**
  - **Linux:** X11 via runtime `dlopen("libX11.so.6")` (`crates/droplet-platform/src/linux_x11.rs`).
  - **Windows:** Win32 windowing + `StretchDIBits` (`crates/droplet-platform/src/windows.rs`).
  - **macOS:** native Cocoa slot (`crates/droplet-platform/src/macos.rs`, requires a macOS host with the Apple SDK).

---

## Repository Layout

```text
apps/droplet-desktop/     CLI entry point + headless dump/screenshot modes
crates/droplet-url/       URL parser and resolver
crates/droplet-net/       HTTP/HTTPS, DNS cache, keep-alive pool, tracker filter, about:* pages
crates/droplet-dom/       Arena DOM + iterative HTML parser
crates/droplet-style/     CSS parser, cascade, Copy ComputedStyle
crates/droplet-layout/    Block/inline/flex layout, hit testing
crates/droplet-gfx/       Canvas, colors, geometry, shared font/image types
crates/droplet-font/      TrueType discovery, sfnt/glyf rasterizer, glyph atlas
crates/droplet-image/     PNG/JPEG/BMP/PPM/SVG decoders
crates/droplet-paint/     Flat display list, chrome and overlays
crates/droplet-platform/  X11/Wayland/Win32/macOS windowing backends
crates/droplet-shell/     Multi-tab browser session, loader and interaction state
```

The old monolithic `src/` tree is kept only as a historical reference; the
workspace above is the canonical build. `Cargo.toml` is a virtual workspace, so
`cargo test` / `cargo clippy` operate on every crate, while `cargo run -- ...`
runs the `droplet-desktop` binary.

---

## Quick Start

### Prerequisites
- Rust toolchain (`/home/mrevds/.cargo/bin/cargo` / `rustc`)
- **Linux GUI:** X11 (`libX11.so.6`); `libssl.so.4`/`.3`/`.so` for `https://`
- **Windows GUI:** Win32 (`user32.dll`, `gdi32.dll`)
- Headless CLI modes (`--dump-dom`, `--dump-layout`, `--screenshot`) run without any display server.

### Build & Test

```bash
/home/mrevds/.cargo/bin/cargo test
/home/mrevds/.cargo/bin/cargo clippy --all-targets
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
```

### Built-in `about:*` Pages
- `about:droplet` — Welcome page & navigation hub
- `about:roadmap` — Phased development plan
- `about:architecture` — Rendering pipeline & Arena DOM overview
- `about:test` — Box Model, Flexbox, Cyrillic/ASCII typography, links, lists & `<pre><code>` visual test page
- `about:blank` — Empty page

---

## Controls (GUI Window)

| Input | Action |
| --- | --- |
| **Left Click** | Activate `<a href>` links, switch/close tabs, click `+` new tab, or toolbar buttons (`<`, `>`, `R`, Home, address bar) |
| **Mouse Wheel** | Scroll page up / down |
| **Mouse Buttons 8 / 9** | History Back / Forward |
| `Ctrl+T` | Open a new tab |
| `Ctrl+W` | Close active tab |
| `Ctrl+Tab` | Cycle to next tab |
| `Ctrl+L` | Focus address bar |
| `Enter` | Navigate to address bar URL (when focused) |
| `Esc` | Unfocus address bar |
| `Ctrl+R` or `F5` | Reload current page |
| `Alt+Left` / `Alt+Right` | History Back / Forward |
| `Up` / `Down` / `PageUp` / `PageDown` / `Home` / `End` / `Space` | Scroll page |

---

## Documentation

- [ARCHITECTURE.md](ARCHITECTURE.md) — End-to-end rendering pipeline, module map, cross-platform layer, and Low-RAM invariants.
- [ROADMAP.md](ROADMAP.md) — Phased development roadmap & Cross-Platform Windowing Strategy (in Russian with standard technical terms in English).
- [CLAUDE.md](CLAUDE.md) — Engineering standards, Linux + Windows rules, and autonomous workflow instructions for AI agents.
- [.scratch/SPEC.md](.scratch/SPEC.md) — Frontier issue tracker (`DROP-001` .. `DROP-011`).
