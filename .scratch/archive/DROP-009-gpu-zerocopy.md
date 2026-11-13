# DROP-009: Hardware GPU / Zero-Copy Surface Blitting & Dirty Region Present

- **Phase:** 8a
- **Status:** `done`
- **Blocked By:** `DROP-006` (`done`)
- **Implementation owner:** Swarm Wave 2 (`tools`, `wayland`, `winmac-jpeg`)

---

## Goal

Eliminate redundant CPU framebuffer copies during scrolling and interaction by moving frame presentation to zero-copy / shared-memory native surfaces across platforms (X11 `MIT-SHM`, Win32 `DIBSection` + optional Direct2D/present polish, Wayland `wl_shm` damage tracking) plus dirty-rect tracking in `droplet-gfx` / `droplet-paint` / `droplet-platform`, keeping zero external crate dependencies.

## Required scope

1. **Dirty-Rectangle / Partial Damage Contract (`crates/droplet-gfx`, `crates/droplet-paint`, `crates/droplet-platform`):**
   - Add a compact `DamageRect` / `DirtyRegion` helper (`{ x, y, width, height }` + union/clamp to viewport, full-frame fallback).
   - Allow `PlatformApp` / `Browser` to report damaged bounds (or render straight into a caller-provided slice `&mut [u32]` / `CanvasView` backed by platform shared memory so no extra `Vec<u32>` copy happens before present).
2. **Linux X11 `MIT-SHM` Zero-Copy Blit (`crates/droplet-platform/src/linux_x11.rs`):**
   - Dynamically probe `libXext.so.6` via `dlopen` (`XShmQueryExtension`, `XShmCreateImage`, `XShmAttach`, `XShmPutImage`, `XShmDetach`) + POSIX `shmget`/`shmat`/`shmdt`/`shmctl(IPC_RMID)` (or `memfd` where applicable, cleanly marked `IPC_RMID` right after attach so no orphan SysV SHM segments leak on crash).
   - Allocate shared XRGB pixel memory once per window size; render **directly** into the SHM-mapped buffer and present via `XShmPutImage` (passing damaged sub-rect when only part of the frame changed!).
   - Graceful automatic fallback to standard `XCreateImage` + `XPutImage` when `libXext.so.6` or XShm extension is unavailable (e.g., remote X11/Xvfb without SHM).
3. **Wayland `wl_shm` Damage Optimization (`crates/droplet-platform/src/linux_wayland.rs`):**
   - Ensure double-buffered `wl_shm` buffers render directly in place without intermediate extra copy and emit `wl_surface.damage_buffer` on actual dirty region rather than always damaging full surface when partial damage is known.
4. **Windows `DIBSection` Zero-Copy GDI Blit (`crates/droplet-platform/src/windows.rs`):**
   - Replace per-frame `StretchDIBits` user-buffer copy with a persistent `CreateDIBSection` (`DIB_RGB_COLORS`, top-down `biHeight = -(h as i32)`) + compatible memory `HDC` (`CreateCompatibleDC` / `SelectObject` / `BitBlt`) so `Browser` renders directly into GDI-backed shared scanlines and presents damaged rects via `BitBlt` with zero heap->kernel copy per frame.
   - Ensure clean `DeleteObject` / `DeleteDC` teardown on resize and window close, and keep `cargo check --target x86_64-pc-windows-gnu --all-targets` and `clippy` cleanly warning-free.
5. **Telemetry & Frame Present Diagnostics (`crates/droplet-shell` / `crates/droplet-paint`):**
   - Expose present backend name (`wayland-shm`, `x11-shm`, `x11-putimage`, `win32-dibsection`, `cocoa`) or subtle timing/backend inspection where helpful without altering core chrome invariants.
   - Keep idle CPU at `~0%` (blocking wait on platform event loops unless active animations/timers require a frame).

## Verification

- Unit tests in `droplet-gfx` and `droplet-platform` covering `DamageRect` union/clamping/empty handling, SHM buffer sizing/stride alignment, graceful fallback probing (non-zero/missing XShm extension), and Windows `DIBSection` struct layout/cleanup compiles cleanly on `x86_64-pc-windows-gnu`.
- Benchmarks or unit timing sanity test confirming `Canvas` direct-slice / view rendering matches `Canvas::new` output pixel-for-pixel.
- Full Global Verification Gate (`cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, cross-target `cargo check`, CLI dump & screenshot).

