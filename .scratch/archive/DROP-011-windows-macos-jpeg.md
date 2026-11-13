# DROP-011b: Windows Schannel TLS, macOS Cocoa Slot & JPEG Conformance

- **Phase:** 9
- **Status:** `done`
- **Blocked By:** `DROP-003`, `DROP-004` (both `done`)
- **Implementation owner:** Swarm Wave 1 (`winmac-jpeg`)

---

## Goal

Finish the non-Linux TLS story, make the macOS native slot real code, and harden
the already-landed baseline JPEG decoder.

## Required scope

1. **Windows Schannel TLS** — new `crates/droplet-net/src/tls_schannel.rs` using
   `secur32.dll`/`crypt32.dll` FFI (`AcquireCredentialsHandleW`,
   `InitializeSecurityContextW`, `QueryContextAttributesW`,
   `EncryptMessage`/`DecryptMessage`, `FreeCredentialsHandle`). Integrate behind
   `#[cfg(target_os = "windows")]` in `tls.rs` with the same read/write API as
   the Linux OpenSSL stream. Validate system certificate trust and SNI.
   Linux builds must stay warning-free (cfg-gated code must be syntactically
   valid). Document that live Windows testing requires a Windows host.
2. **macOS Cocoa** — replace the `macos.rs` stub with a real `cfg(target_os =
   "macos")` implementation outline: `NSApplication`, `NSWindow`, `NSView`,
   `CGContext`/bitmap blit via Objective-C runtime FFI, event loop, keyboard and
   mouse mapping to `PlatformApp`. It cannot be compiled on Ubuntu; ensure the
   file is excluded from Linux builds and keep the error path for unsupported
   targets.
3. **JPEG hardening** — keep baseline DCT decode; add regression tests for
   grayscale, 4:2:0 and 4:4:4 sampling, restart-free/with-DRI files, truncated
   data, progressive (`SOF2`) clean rejection, and dimension bombs. Fix any
   decoder bugs found. Add a small test image builder if needed.
4. **Tests** — Linux `cargo test --workspace` and
   `cargo clippy --workspace --all-targets` must be clean. Windows/macOS code is
   cfg-gated; run `cargo check` for the workspace as the baseline gate.

## Constraints

- Zero dependencies.
- Do not touch `droplet-shell`, `droplet-layout`, forms or history.
- Keep TLS functions OS-gated; never leak `windows-sys`/Schannel types into
  core modules.
