# CLAUDE.md — Agent Guide for Droplet

**Droplet** is a zero-bloat graphical web browser engine written from scratch in pure Rust (`edition = "2021"`) for **Linux (X11)**, **Windows (Win32)**, and headless CLI (plus an architectural macOS slot).  
North star: **maximum speed, zero bloat («очень быстрый браузер без мусора»)**.

---

## Build, Test & Inspection Commands

Cargo is installed at `/home/mrevds/.cargo/bin/cargo` (build artifacts go to `/tmp/droplet-target` via `.cargo/config.toml`).

```bash
# Run full unit & integration test suite
/home/mrevds/.cargo/bin/cargo test

# Lint with Clippy (zero warnings expected)
/home/mrevds/.cargo/bin/cargo clippy --all-targets

# Build release binary
/home/mrevds/.cargo/bin/cargo build --release

# Launch GUI browser (defaults to about:droplet)
/home/mrevds/.cargo/bin/cargo run -- about:droplet

# Headless inspection & screenshot flags
/home/mrevds/.cargo/bin/cargo run -- --dump-dom about:test
/home/mrevds/.cargo/bin/cargo run -- --dump-layout about:test
/home/mrevds/.cargo/bin/cargo run -- --screenshot /tmp/out.ppm about:droplet
```

---

## Architecture & Engineering Principles

1. **Zero Bloat / Zero External Dependencies**
   - Every workspace crate keeps its `[dependencies]` section limited to other local `droplet-*` crates—no external crates. Pure Rust `std` plus runtime dynamic loading (`dlopen` on Linux for `libX11.so.6`, `libwayland-client.so.0`, and `libssl.so.4`/`.3`/`.so`; Win32 `StretchDIBits`/Schannel on Windows).
   - Do not add heavy frameworks, async runtimes (`tokio`), proc-macro graphs, or `Cargo.toml` bloat. Prefer clean pure-Rust implementations (e.g., `sfnt`/`glyf`, PNG/JPEG/BMP/PPM/SVG, `TrackerFilter`, bytecode VM) or zero-link runtime OS bindings.
2. **Linux + Windows Cross-Platform Discipline**
   - Isolate all OS-specific windowing, event loops, framebuffer blitting, and native FFI inside `crates/droplet-platform/src/` (`lib.rs`, `linux_x11.rs`, `linux_wayland.rs`, `windows.rs`, `macos.rs`, `clipboard.rs`) using `#[cfg(target_os = ...)]`. `crates/droplet-platform/src/lib.rs` owns the `PlatformApp`/`NavKey` contract and `run` dispatch.
   - **Never break non-Linux builds:** core pipeline crates (`droplet-url`, `droplet-net`, `droplet-dom`, `droplet-style`, `droplet-layout`, `droplet-image`, `droplet-paint`, `droplet-shell`) must remain 100% OS-agnostic pure Rust `std`—never leak X11/Wayland handles, `RawFd`, POSIX paths, or `dlopen` assumptions into core modules.
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

Before editing any code on a new task:

1. **Read the project roadmap & map:**
   - [ARCHITECTURE.md](ARCHITECTURE.md) — pipeline, modules, exact data structures, and Low-RAM invariants.
   - [ROADMAP.md](ROADMAP.md) — phased plan (Phases 1–9), cross-platform strategy, and performance targets.
   - [.scratch/SPEC.md](.scratch/SPEC.md) — ticket frontier table (`DROP-001` .. `DROP-011`).
2. **Select the next frontier ticket:**
   - Pick a ticket marked `ready-for-agent` whose `Blocked By` tickets are all `done` (currently `DROP-005`, `DROP-006`, `DROP-007`, `DROP-011`).
   - Read its full specification in `.scratch/issues/DROP-*.md` (or draft one if marked `TBD`).
3. **Implement & Verify:**
   - Keep changes strictly scoped to the ticket. Match existing module boundaries and code style.
   - Run `/home/mrevds/.cargo/bin/cargo test` and `/home/mrevds/.cargo/bin/cargo clippy --all-targets`.
   - For visual/layout changes, render `about:test` and `about:droplet` with `--screenshot /tmp/out.ppm` and inspect `--dump-layout`.
4. **Update Tracking Docs Before Finishing:**
   - Mark the ticket `done` in [.scratch/SPEC.md](.scratch/SPEC.md) and update any unblocked downstream ticket statuses (`blocked` -> `ready-for-agent` if all its blockers are now `done`).
   - Update [ROADMAP.md](ROADMAP.md) (and [ARCHITECTURE.md](ARCHITECTURE.md) / [README.md](README.md) if public structs, pipeline stages, shortcuts, or CLI flags changed).
