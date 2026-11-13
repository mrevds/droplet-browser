# DROP-111 — Reliable TLS handshake and timeouts

Phase 1 · Status: ready · Blocked by: — · Owner: _(fill in)_

## Problem
`droplet --dump-dom https://news.ycombinator.com` intermittently fails with
`TLS handshake timed out` after ~7 s while `curl` finishes in ~1.3 s. In
`crates/droplet-net/src/tls.rs` the `SSL_connect` loop treats any `WANT_READ/WANT_WRITE` +
`WouldBlock` as a timeout on a socket that only has `set_read_timeout(5 s)`; the same 5 s
`HTTP_TIMEOUT` covers connect, handshake and every read.

## Design
- Separate deadlines: connect (≤ 5 s, try every resolved address, IPv4 and IPv6, happy-eyeballs-lite:
  next address after 250 ms), handshake (≤ 10 s total), first byte (≤ 15 s), idle read (≤ 15 s).
- Handshake loop: on `WANT_READ`/`WANT_WRITE` wait for readiness against the **remaining**
  deadline and retry; only `Instant::now() >= deadline` is a timeout. On Linux wait with `poll()`
  declared via a small `extern "C"` binding (libc is always linked by std; no new crate), shrinking
  the socket read timeout to the remaining deadline. Keep the Windows Schannel path compiling.
- Errors say which phase failed: `connect`, `handshake`, `read`.

## Acceptance
- Unit test with a local TCP server that accepts and waits 1 s before speaking TLS bytes → no
  spurious timeout; one that never responds → `handshake` timeout error within deadline + 10%.
- Unit test: first address refuses, second accepts → connection succeeds.
- `tools/bench-sites.sh` HN line succeeds 10 runs out of 10 (paste output).
- `cargo clippy --workspace --all-targets` and a Windows `cargo check --target x86_64-pc-windows-gnu`
  if the target is installed (otherwise state that it was not checked).
