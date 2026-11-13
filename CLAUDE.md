# CLAUDE.md — Agent Guide for Droplet

**Droplet** is a zero-bloat graphical web browser engine written from scratch in pure Rust (`edition = "2021"`) for **Linux (X11)**, **Windows (Win32)**, and headless CLI (plus an architectural macOS slot).  
North star: **maximum speed, zero bloat («очень быстрый браузер без мусора»)**.

---

## Build, Test & Inspection Commands

Cargo is installed at `/home/mrevds/.cargo/bin/cargo` (build artifacts go to `/tmp/droplet-target` via `.cargo/config.toml`).

```bash
# Run full unit & integration test suite (includes corpus layout goldens)
/home/mrevds/.cargo/bin/cargo test --workspace

# Lint with Clippy (zero warnings expected), also with the experimental sandbox feature
/home/mrevds/.cargo/bin/cargo clippy --workspace --all-targets
/home/mrevds/.cargo/bin/cargo clippy -p droplet-desktop --features sandbox --all-targets

# Corpus goldens only / re-bless after reviewing screenshots
/home/mrevds/.cargo/bin/cargo test -p droplet-desktop --test corpus
DROPLET_BLESS=1 /home/mrevds/.cargo/bin/cargo test -p droplet-desktop --test corpus

# Per-stage benchmark (median ms per stage + peak RSS); real sites via tools/bench-sites.sh
/home/mrevds/.cargo/bin/cargo run --release -- --bench --runs 20 about:droplet

# Build release binary
/home/mrevds/.cargo/bin/cargo build --release

# Launch GUI browser (defaults to about:droplet)
/home/mrevds/.cargo/bin/cargo run -- about:droplet

# Headless inspection & screenshot flags
/home/mrevds/.cargo/bin/cargo run -- --dump-dom about:test
/home/mrevds/.cargo/bin/cargo run -- --dump-layout about:test
/home/mrevds/.cargo/bin/cargo run -- --screenshot /tmp/out.ppm about:droplet

# Experimental, off by default: ES-subset JS VM (`--js`), sandboxed renderer (`--features sandbox`)
/home/mrevds/.cargo/bin/cargo run -- --js about:test
```

---

## Architecture & Engineering Principles

1. **Zero Bloat / Zero External Dependencies**
   - Every workspace crate keeps its `[dependencies]` section limited to other local `droplet-*` crates—no external crates. Pure Rust `std` plus runtime dynamic loading (`dlopen` on Linux for `libX11.so.6`, `libwayland-client.so.0`, `libssl.so.4`/`.3`/`.so`, and—only inside the `droplet-webview` helper process—GTK 3 + WebKitGTK 4.1 for script-heavy sites, see DROP-170; Win32 `StretchDIBits`/Schannel on Windows).
   - Do not add heavy frameworks, async runtimes (`tokio`), proc-macro graphs, or `Cargo.toml` bloat. Prefer clean pure-Rust implementations (e.g., `sfnt`/`glyf`, PNG/JPEG/BMP/PPM/SVG, `TrackerFilter`, bytecode VM) or zero-link runtime OS bindings.
2. **Linux + Windows Cross-Platform Discipline**
   - Isolate all OS-specific windowing, event loops, framebuffer blitting, and native FFI inside `crates/droplet-platform/src/` (`lib.rs`, `linux_x11.rs`, `linux_wayland.rs`, `windows.rs`, `macos.rs`, `clipboard.rs`) using `#[cfg(target_os = ...)]`. `crates/droplet-platform/src/lib.rs` owns the `PlatformApp`/`NavKey` contract and `run` dispatch.
   - **Never break non-Linux builds:** core pipeline crates (`droplet-url`, `droplet-net`, `droplet-dom`, `droplet-style`, `droplet-layout`, `droplet-inflate`, `droplet-image`, `droplet-paint`, `droplet-shell`) must remain 100% OS-agnostic pure Rust `std`—never leak X11/Wayland handles, `RawFd`, POSIX paths, or `dlopen` assumptions into core modules.
   - OS-specific behavior in shared modules (such as TLS in `crates/droplet-net/src/lib.rs` or system font candidate paths in `crates/droplet-font/src/lib.rs`) must be cleanly gated with `#[cfg(target_os = ...)]` and compile cleanly on both Linux and Windows (and `macos.rs` stub). Note that Cocoa/Metal (`crates/droplet-platform/src/macos.rs`) cannot be built or live-tested natively on Ubuntu without macOS/Apple SDK.
3. **Low-RAM & Cache-Friendly Data Structures**
   - **Keep `ComputedStyle` strictly `Copy`:** zero `HashMap`, `Vec`, or `String` fields inside `ComputedStyle`.
   - **Keep `ElementData.attributes` as `Vec<(String, String)>`:** never replace it with `HashMap<String, String>`—attribute-less elements must incur zero heap allocation.
   - **Arena DOM (`Vec<Node>` + `NodeId = usize`):** never introduce `Rc<RefCell<...>>` or pointer-chasing trees; keep all DOM traversals **iterative** (explicit `Vec` stack) so deep HTML cannot overflow the stack.
   - **Bounded Caches & Hibernation:** preserve hard caps on `GlyphAtlas` (`MAX_ATLAS_ENTRIES = 4096`), **Tab Hibernation** (`MAX_WARM_TABS = 3`), image dimensions (`MAX_IMAGE_PIXELS = 64_000_000`), response size (`16 MiB`), DNS cache (`256`), and Keep-Alive sockets (`32`).
4. **Flat Display List & Dirty-Flag Rendering**
   - Layout computes explicit `LayoutBox` geometry; paint builds a flat `DisplayList` culled in O(1) by viewport Y bounds.
   - Keep `:hover`/`:active` restyling gated by `stylesheet_uses_interaction(&stylesheet)` so pages without interactive selectors incur zero mouse-motion layout cost. Keep idle CPU near `0%`.
5. **Benchmark-Driven Performance & UTF-8 Safety**
   - Target metrics: cold start `< 20 ms`, idle RAM `< 15 MB` (single tab) / tens of MB multi-tab, layout + paint `< 2 ms` on typical pages.
   - Every string parser (`url`, `html`, `css`, text wrapping, address bar editing) must respect UTF-8 character boundaries. Test Cyrillic and multi-byte Unicode in every parser/renderer change.
6. **Tests Required for Every Change**
   - Every new feature or bugfix starts with unit/regression tests in the affected module (`#[cfg(test)] mod tests`) plus headless CLI verification (`--dump-dom`, `--dump-layout`, `--screenshot`) when UI/rendering changes.

---

## Mandatory Agent Workflow

**Depth before breadth.** The project was re-baselined in Nov 2026 because earlier agents marked
tickets `done` when tests passed while real pages rendered wrong. "Done" now means *seen on a
screenshot and measured*.

Before editing any code on a new task:

1. **Read:** [ROADMAP.md](ROADMAP.md) (phases + rules), [.scratch/SPEC.md](.scratch/SPEC.md) (ticket
   table + done gate), [ARCHITECTURE.md](ARCHITECTURE.md) (pipeline, Low-RAM invariants),
   [tests/corpus/README.md](tests/corpus/README.md) (golden workflow).
2. **Pick a ticket:** the lowest-numbered `ready` ticket in the earliest unfinished phase. Never work
   on `frozen` code (`droplet-js`, `droplet-sandbox`, `droplet-ipc`, `linux_wayland.rs`, `macos.rs`)
   beyond build fixes. Write your name/model into the ticket's `Owner` line.
3. **Measure first:** run `--bench` on the ticket's pages and keep the numbers.
4. **Implement & verify** on a branch, scoped to the ticket:
   - tests first in the affected module (`#[cfg(test)] mod tests`), Cyrillic/UTF-8 included;
   - `cargo test --workspace`, both clippy commands above;
   - screenshot the ticket's corpus pages, compare with the page's `EXPECT:` comment, flip
     `tests/corpus/manifest.txt` to `ok` only if it is right, re-bless goldens and read the diff;
   - `--bench` after: not worse; paste before/after numbers in the commit message.
5. **Update tracking docs:** ticket status in `.scratch/SPEC.md` (+ unblock downstream), the metrics
   table in `ROADMAP.md` if a number moved, `ARCHITECTURE.md` / `README.md` if public structs,
   pipeline stages, CLI flags or shortcuts changed.
6. **Commit** per ticket. Never leave work uncommitted, never run several agents in one dirty checkout.
