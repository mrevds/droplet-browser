# Droplet Execution Frontier (`SPEC.md`)

This file is the source of truth for autonomous agents picking up work on **Droplet**.  
Before starting a ticket:
1. Read [CLAUDE.md](../CLAUDE.md), [ARCHITECTURE.md](../ARCHITECTURE.md), and [ROADMAP.md](../ROADMAP.md).
2. Choose a ticket in `ready-for-agent` status whose `Blocked By` dependencies are all `done`.
3. Open its spec in `.scratch/issues/` (if present) and verify all acceptance tests after implementation.

---

## Frontier Table (`DROP-001` .. `DROP-011`)

| ID | Phase | Title | Status | Blocked By | Owner / Model | Spec | Verification Command |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `DROP-001` | Phase 1 | Core Pipeline (URL, HTTP/1.1, HTML/DOM, CSS/Style, Layout, 8x16 Font, Rasterizer, X11/PPM) | `done` | — |  DeepSeek V4.1 Flash Free (OpenCode agent)  | Core v0.1 baseline | `/home/mrevds/.cargo/bin/cargo test && /home/mrevds/.cargo/bin/cargo run -- --screenshot /tmp/droplet-p1.ppm about:test` |
| `DROP-002` | Phase 2 | Full Inline-Block & Flexbox Layout, Relative Units (`%`/`em`/`rem`/`vh`/`vw`), `:hover`/`:active` | `done` | `DROP-001` |  DeepSeek V4.1 Flash Free (OpenCode agent)  | [DROP-002](issues/DROP-002-inline-and-flexbox.md) | `/home/mrevds/.cargo/bin/cargo test && /home/mrevds/.cargo/bin/cargo run -- --dump-layout about:test` |
| `DROP-003` | Phase 3 | HTTPS (Linux OpenSSL TLS), Keep-Alive Pool, Concurrent External CSS Fetch, DNS Cache & Tracker Filter | `done` | `DROP-001` |  DeepSeek V4.1 Flash Free (OpenCode agent)  | [DROP-003](issues/DROP-003-https-and-external-css.md) | `/home/mrevds/.cargo/bin/cargo test && /home/mrevds/.cargo/bin/cargo clippy --all-targets` |
| `DROP-004` | Phase 4 | System TTF Discovery (`sfnt`/`glyf` AA + Atlas), PNG/BMP/PPM/SVG Decoding, `<img>` Boxes, Multi-Tab & Win32 Shell | `done` | `DROP-001` |  DeepSeek V4.1 Flash Free (OpenCode agent)  | [DROP-004](issues/DROP-004-truetype-fonts-and-images.md) | `/home/mrevds/.cargo/bin/cargo test && /home/mrevds/.cargo/bin/cargo run -- --screenshot /tmp/droplet-p4.ppm about:droplet` |
| `DROP-005` | Phase 5 | HTML Forms (`<input>`, `<textarea>`, `<button>`, `<select>`), Focus Ring, Text Selection & X11/Win32 Clipboard | `done` | `DROP-002` |  Swarm Wave 1 (`forms`)  | [DROP-005](issues/DROP-005-forms-and-selection.md) | `/home/mrevds/.cargo/bin/cargo test && /home/mrevds/.cargo/bin/cargo clippy --all-targets` |
| `DROP-006` | Phase 6 | Persistent Bookmarks/History Store, Find-in-Page (`Ctrl+F`), Keyboard-First Command UI & Box Model DevTools | `done` | `DROP-002`, `DROP-004` |  Swarm Wave 1 (`tools`)  | [DROP-006](issues/DROP-006-shell-tools.md) | `/home/mrevds/.cargo/bin/cargo test && /home/mrevds/.cargo/bin/cargo clippy --all-targets` |
| `DROP-007` | Phase 7 | Minimalist ES Subset Parser & Register/Stack Bytecode VM | `done` | `DROP-002`, `DROP-003` |  Swarm Wave 1 (`js-vm`)  | [DROP-007](issues/DROP-007-js-vm.md) | `/home/mrevds/.cargo/bin/cargo test && /home/mrevds/.cargo/bin/cargo clippy --all-targets` |
| `DROP-008` | Phase 7 | Reactive DOM Bindings, Event Dispatch & Incremental Dirty Relayout/Repaint | `done` | `DROP-005`, `DROP-007` |  Swarm Wave 2 (`js-vm`, `forms`)  | [DROP-008](issues/DROP-008-reactive-dom.md) | `/home/mrevds/.cargo/bin/cargo test --workspace && /home/mrevds/.cargo/bin/cargo clippy --workspace --all-targets` |
| `DROP-009` | Phase 8 | Hardware GPU Rendering / Zero-Copy Blitting (X11 `MIT-SHM`, Win32 `DIBSection` & Damage Tracking) | `done` | `DROP-006` |  Swarm Wave 2 (`tools`, `wayland`, `winmac-jpeg`)  | [DROP-009](issues/DROP-009-gpu-zerocopy.md) | `/home/mrevds/.cargo/bin/cargo test --workspace && /home/mrevds/.cargo/bin/cargo clippy --workspace --all-targets` |
| `DROP-010` | Phase 8 | Multi-Process Renderer Isolation (Linux `seccomp-bpf` + Namespaces + `memfd` IPC / Windows Job & AppContainer) | `done` | `DROP-008`, `DROP-009` |  Swarm Wave 3 (`wayland`, `js-vm`, `forms`, `tools`, `winmac-jpeg`)  | [DROP-010](issues/DROP-010-multiprocess-sandbox.md) | `/home/mrevds/.cargo/bin/cargo test --workspace && /home/mrevds/.cargo/bin/cargo clippy --workspace --all-targets` |
| `DROP-011` | Phase 9 | Native Wayland Client, Windows Schannel TLS, Pure-Rust Baseline JPEG Decoder & macOS Cocoa (when on macOS) | `done` | `DROP-003`, `DROP-004` |  Swarm Wave 1 (`wayland`, `winmac-jpeg`)  | [Wayland](issues/DROP-011-wayland.md), [Win/macOS/JPEG](issues/DROP-011-windows-macos-jpeg.md) | `/home/mrevds/.cargo/bin/cargo test && /home/mrevds/.cargo/bin/cargo clippy --all-targets` |

---

## Model / Agent Attribution (multi-agent project)

The `Owner / Model` column above records which AI model/agent implemented each phase, because Droplet phases are executed by several agents in parallel:

- **DeepSeek V4.1 Flash Free (OpenCode agent)** — implemented the completed baseline: `DROP-001` … `DROP-004` (core pipeline, flexbox/inline-block, HTTPS/OpenSSL + tracker filter, TTF fonts + image decoders + multi-tab shell).
- **Cline (AI coding agent)** — currently executing the open frontier: `DROP-005`, `DROP-006`, `DROP-007`, `DROP-011`. Each agent keeps its changes strictly scoped to its ticket(s) and respects module boundaries so parallel work does not collide.
- **Unassigned** (`—`) — tickets whose blockers are not yet done (`DROP-008`, `DROP-009`, `DROP-010`); they become claimable once all `Blocked By` tickets reach `done`.

Before starting a ticket, an agent updates its issue file's `Implementation owner` line to its own name/model. After finishing, it marks the ticket `done` in this table and flips any downstream `blocked` ticket to `ready-for-agent` when all its blockers are `done`.

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
