# DROP-011a: Native Wayland Client with X11 Fallback

- **Phase:** 9
- **Status:** `in-progress`
- **Blocked By:** `DROP-003`, `DROP-004` (both `done`)
- **Implementation owner:** DeepSeek V4.1 Flash Free (OpenCode agent)

---

## Goal

A native Wayland windowing backend loaded at runtime, with transparent fallback
to the existing X11 backend when no compositor is available.

## Required scope

1. **Dynamic loading** — `dlopen("libwayland-client.so.0")`; loader must fail
   gracefully into X11 when the library, `WAYLAND_DISPLAY`, or
   `XDG_RUNTIME_DIR` is missing.
2. **Protocol client** — hand-written wire marshalling for `wl_display`,
   `wl_registry`, `wl_compositor`, `wl_shm`, `wl_shm_pool`, `wl_buffer`,
   `wl_seat`, `wl_keyboard`, `wl_pointer`, `xdg_wm_base`, `xdg_surface`,
   `xdg_toplevel`; roundtrip sync.
3. **Shared-memory presentation** — `memfd_create`/`ftruncate`/`mmap` via FFI,
   `wl_shm` buffer pool (double buffering), ARGB8888/XRGB8888 format, resize,
   `wl_buffer.release` recycling, damage/commit, close/ping handling.
4. **Input** — keyboard (text input, modifiers, Enter/Backspace/Esc, arrows,
   PageUp/PageDown/Home/End, Ctrl shortcuts) and pointer (motion, button,
   axis) mapped onto `PlatformApp` / `NavKey`.
5. **Fallback** — `platform::run` prefers Wayland only when it can connect and
   initialize; otherwise falls back to X11 and reports a useful error if both
   fail. Linux-only `#[cfg]`, no X11 types in shared modules.
6. **Tests** — deterministic unit tests for wire encoding/decoding, registry
   state, fallback decision (`WAYLAND_DISPLAY` / library availability), key
   mapping and buffer-pool state. Live compositor testing is documented as
   manual where unavailable.

## Constraints

- Zero dependencies; raw FFI declarations only.
- No unsafe code outside `crates/droplet-platform/src/linux_wayland.rs` and the
  existing platform backend module.
- Idle CPU stays ~0% (blocking `wl_display_dispatch`).
- Touch no shell/paint/layout files.
