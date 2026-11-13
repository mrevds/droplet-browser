# DROP-010: Multi-Process Renderer Isolation (`seccomp-bpf` + Namespaces + `memfd` IPC / Win32 Job & Restricted Token)

- **Phase:** 8b
- **Status:** `done`
- **Blocked By:** `DROP-008` (`done`), `DROP-009` (`done`)
- **Implementation owner:** Swarm Wave 3 (`wayland`, `js-vm`, `forms`, `tools`, `winmac-jpeg`)

---

## Goal

Separate the privileged Browser/UI process from untrusted HTML/CSS/Image/JS Renderer execution using pure-Rust OS sandboxing (`crates/droplet-sandbox`), binary + shared-memory IPC (`crates/droplet-ipc`), and child `--renderer` supervision in `apps/droplet-desktop`, with zero external crate dependencies and clean cross-compilation to Linux, Windows (`x86_64-pc-windows-gnu`), and macOS (`aarch64-apple-darwin`).

## Required scope

1. **Pure-Rust OS Sandbox (`crates/droplet-sandbox`):**
   - Public API: `SandboxProfile { StrictRenderer, PermissiveFallback }`, `SandboxReport`, `SandboxError`, `apply_renderer_sandbox(profile: SandboxProfile) -> Result<SandboxReport, SandboxError>`.
   - **Linux (`linux.rs`):**
     - Resource limits (`setrlimit` for `RLIMIT_CORE = 0`, `RLIMIT_NOFILE`, bounded address space) + `prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0)`.
     - Best-effort `unshare(CLONE_NEWUSER | CLONE_NEWNET)` + uid/gid map (`setgroups="deny"`), recording accurate boolean status in `SandboxReport` without crashing if kernel disables unprivileged user namespaces.
     - Pure-Rust **seccomp-bpf** bytecode filter (`sock_filter` / `sock_fprog` verified for x86_64 & aarch64) installed via `prctl(PR_SET_SECCOMP, SECCOMP_MODE_FILTER, ...)` / `SYS_seccomp`, blocking `socket`, `connect`, `bind`, `open`/`openat` write/network syscalls, `execve`/`execveat`, `clone`/`fork` with `EPERM`, while permitting allocator/futex/pipe/clock/exit syscalls needed by renderer.
     - Real `fork()`-isolated unit tests verifying sandbox enforcement (`socket` fails with `EPERM`, allowed computation + pipe `write` succeed).
   - **Windows (`windows.rs`):**
     - Win32 Job Object (`CreateJobObjectW`, `SetInformationJobObject` `ExtendedLimitInformation` + `BasicUiRestrictions`, `AssignProcessToJobObject`) + Restricted Token (`OpenProcessToken`, `CreateRestrictedToken` + `DISABLE_MAX_PRIVILEGE`) + structural/limit unit tests.
   - **macOS (`macos.rs`):**
     - `rlimit` + `sandbox_init` network/filesystem denial FFI compiling cleanly under `aarch64-apple-darwin`.

2. **Zero-Copy Shared-Memory & Binary Pipe IPC (`crates/droplet-ipc`):**
   - `ShmFrameBuffer`: Linux `memfd_create` / `ftruncate` / `mmap` / FD inheritance (`CLOEXEC` clear for child pass); Windows `CreateFileMappingW` / `MapViewOfFile` / portable backing.
   - `protocol.rs`: versioned binary length-prefixed framing (`MAX_IPC_MESSAGE_BYTES = 32 MiB`), `BrowserToRenderer` (`RenderPage`, `Scroll`, `Resize`, `MouseMove`, `MouseDown`, `MouseUp`, `JsEval`, `Shutdown`), `RendererToBrowser` (`Ready`, `FrameReady`, `ConsoleLog`, `NavigateRequest`, `Error`, `Ack`). Strict bounds checks rejecting corrupt/oversized frames.
   - `channel.rs`: bidirectional pipe endpoints + multi-process & in-memory round-trip tests.

3. **Shell Hooks (`crates/droplet-shell`):**
   - Wire `DROP-009` `DamageRect`, `render_frame_damaged`, `PlatformApp::take_damage`, and `PlatformApp::set_present_backend` into `Browser`.
   - Expose `HeadlessRenderer` / `RenderResult` cleanly for rendering into a caller-provided pixel slice (`&mut [u32]`) under pre-fetched HTML/CSS/image payloads so a sandboxed child renderer needs **zero** direct network or filesystem syscalls!

4. **Multi-Process Supervisor & `--renderer` Mode (`apps/droplet-desktop`):**
   - `--renderer` child mode: drops privileges via `droplet_sandbox::apply_renderer_sandbox(SandboxProfile::StrictRenderer)`, runs IPC command loop reading `BrowserToRenderer` and painting into inherited `ShmFrameBuffer`.
   - `RendererHost` supervisor in `droplet-desktop` + `--sandboxed` / `--sandbox-check` or headless sandboxed render path: spawns child `--renderer`, passes pre-fetched payload + SHM FD, receives rendered pixels, and handles child crash/EOF gracefully without panicking the host process.

## Verification

- `cargo test --workspace` (including `droplet-sandbox` `fork` enforcement test, `droplet-ipc` protocol & `memfd_create` tests, `droplet-shell` damaged rendering tests, and `droplet-desktop` end-to-end multi-process sandboxed render test!).
- `cargo clippy --workspace --all-targets -- -D warnings`.
- `cargo check --target x86_64-pc-windows-gnu --workspace --all-targets` and `cargo check --target aarch64-apple-darwin --workspace --all-targets`.
- Global headless CLI checks (`--dump-dom`, `--dump-layout`, `--screenshot`).

