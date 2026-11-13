# Droplet Execution Frontier (`SPEC.md`)

This file is the source of truth for autonomous agents picking up work on **Droplet**.  
Before starting a ticket:
1. Read [CLAUDE.md](../CLAUDE.md), [ARCHITECTURE.md](../ARCHITECTURE.md), and [ROADMAP.md](../ROADMAP.md).
2. Choose a ticket in `ready-for-agent` status whose `Blocked By` dependencies are all `done`.
3. Open its spec in `.scratch/issues/` (if present) and verify all acceptance tests after implementation.

---

## Frontier Table (`DROP-001` .. `DROP-011`)

| ID | Phase | Title | Status | Blocked By | Spec | Verification Command |
| --- | --- | --- | --- | --- | --- | --- |
| `DROP-001` | Phase 1 | Core Pipeline (URL, HTTP/1.1, HTML/DOM, CSS/Style, Layout, 8x16 Font, Rasterizer, X11/PPM) | `done` | — | Core v0.1 baseline | `/home/mrevds/.cargo/bin/cargo test && /home/mrevds/.cargo/bin/cargo run -- --screenshot /tmp/droplet-p1.ppm about:test` |
| `DROP-002` | Phase 2 | Full Inline-Block & Flexbox Layout, Relative Units (`%`/`em`/`rem`/`vh`/`vw`), `:hover`/`:active` | `done` | `DROP-001` | [DROP-002](issues/DROP-002-inline-and-flexbox.md) | `/home/mrevds/.cargo/bin/cargo test && /home/mrevds/.cargo/bin/cargo run -- --dump-layout about:test` |
| `DROP-003` | Phase 3 | HTTPS (Linux OpenSSL TLS), Keep-Alive Pool, Concurrent External CSS Fetch, DNS Cache & Tracker Filter | `done` | `DROP-001` | [DROP-003](issues/DROP-003-https-and-external-css.md) | `/home/mrevds/.cargo/bin/cargo test && /home/mrevds/.cargo/bin/cargo clippy --all-targets` |
| `DROP-004` | Phase 4 | System TTF Discovery (`sfnt`/`glyf` AA + Atlas), PNG/BMP/PPM/SVG Decoding, `<img>` Boxes, Multi-Tab & Win32 Shell | `done` | `DROP-001` | [DROP-004](issues/DROP-004-truetype-fonts-and-images.md) | `/home/mrevds/.cargo/bin/cargo test && /home/mrevds/.cargo/bin/cargo run -- --screenshot /tmp/droplet-p4.ppm about:droplet` |
| `DROP-005` | Phase 5 | HTML Forms (`<input>`, `<textarea>`, `<button>`, `<select>`), Focus Ring, Text Selection & X11/Win32 Clipboard | `in-progress` | `DROP-002` | [DROP-005](issues/DROP-005-forms-and-selection.md) | `/home/mrevds/.cargo/bin/cargo test && /home/mrevds/.cargo/bin/cargo clippy --all-targets` |
| `DROP-006` | Phase 6 | Persistent Bookmarks/History Store, Find-in-Page (`Ctrl+F`), Keyboard-First Command UI & Box Model DevTools | `in-progress` | `DROP-002`, `DROP-004` | [DROP-006](issues/DROP-006-shell-tools.md) | `/home/mrevds/.cargo/bin/cargo test && /home/mrevds/.cargo/bin/cargo clippy --all-targets` |
| `DROP-007` | Phase 7 | Minimalist ES Subset Parser & Register/Stack Bytecode VM | `in-progress` | `DROP-002`, `DROP-003` | [DROP-007](issues/DROP-007-js-vm.md) | `/home/mrevds/.cargo/bin/cargo test && /home/mrevds/.cargo/bin/cargo clippy --all-targets` |
| `DROP-008` | Phase 7 | Reactive DOM Bindings, Event Dispatch & Incremental Dirty Relayout/Repaint | `blocked` | `DROP-005`, `DROP-007` | TBD (draft before starting) | `/home/mrevds/.cargo/bin/cargo test` |
| `DROP-009` | Phase 8 | Hardware GPU Rendering / Zero-Copy Blitting (X11 `MIT-SHM`, Win32 `DIBSection`/Direct2D, or `wgpu`/Vulkan) | `blocked` | `DROP-006` | TBD (draft before starting) | `/home/mrevds/.cargo/bin/cargo test` |
| `DROP-010` | Phase 8 | Multi-Process Renderer Isolation (Linux `seccomp-bpf` + Namespaces + `memfd` IPC / Windows Job & AppContainer) | `blocked` | `DROP-008`, `DROP-009` | TBD (draft before starting) | `/home/mrevds/.cargo/bin/cargo test` |
| `DROP-011` | Phase 9 | Native Wayland Client, Windows Schannel TLS, Pure-Rust Baseline JPEG Decoder & macOS Cocoa (when on macOS) | `in-progress` | `DROP-003`, `DROP-004` | [Wayland](issues/DROP-011-wayland.md), [Win/macOS/JPEG](issues/DROP-011-windows-macos-jpeg.md) | `/home/mrevds/.cargo/bin/cargo test && /home/mrevds/.cargo/bin/cargo clippy --all-targets` |

---

## Dependency Notes

- **Completed baseline (`DROP-001` .. `DROP-004`):** Core pipeline, Flexbox/InlineBlock/`vh`/`vw`/`:hover`/`:active`, Linux OpenSSL HTTPS + Keep-Alive + `TrackerFilter`, pure-Rust `sfnt`/`glyf` AA font engine + `GlyphAtlas`, PNG/BMP/PPM/SVG decoder + `<img>` boxes, multi-tab `TabState` + Tab Hibernation (`MAX_WARM_TABS = 3`), and `src/platform/` (`linux_x11.rs` + `windows.rs` + `macos.rs` stub) are implemented.
- **Current open frontier (`ready-for-agent`):** `DROP-005`, `DROP-006`, `DROP-007`, and `DROP-011` all have their blockers satisfied and may be executed in parallel (watching module boundaries).
- **Updated blocker notes:** `DROP-009` remains blocked only by `DROP-006` (`DROP-004` is `done`); `DROP-010` remains blocked only by `DROP-008`, `DROP-009` (`DROP-003` is `done`).

---

## Global Verification Gate (Every Ticket)

```bash
/home/mrevds/.cargo/bin/cargo test
/home/mrevds/.cargo/bin/cargo clippy --all-targets
/home/mrevds/.cargo/bin/cargo run -- --dump-dom about:test
/home/mrevds/.cargo/bin/cargo run -- --dump-layout about:test
/home/mrevds/.cargo/bin/cargo run -- --screenshot /tmp/droplet-verify.ppm about:droplet
```
