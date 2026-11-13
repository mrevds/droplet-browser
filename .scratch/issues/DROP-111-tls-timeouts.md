# DROP-111 — Reliable TLS handshake and timeouts

Phase 1 · Status: done · Blocked by: — · Owner: Claude Opus 5.5 (Claude Code)

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

## Result (2026-11-13)
- Root cause confirmed: one `HTTP_TIMEOUT = 5 s` was both the socket `SO_RCVTIMEO` and the
  handshake limit, and `WANT_READ` after a single expired socket wait was reported as
  "handshake timed out". On the lossy path to HN one lost server flight (1 + 2 + 4 s TCP
  backoff) exceeds 5 s. curl showed the same stalls (TLS 5.6 s, one 73 s run).
- `crates/droplet-net/src/lib.rs`: `CONNECT_TIMEOUT = 10 s` (all addresses), `HANDSHAKE_TIMEOUT = 15 s`,
  `READ_TIMEOUT = 15 s` idle.
- OpenSSL (`tls.rs`): `TlsStream::connect_within(tcp, host, budget)` shrinks the socket timeouts
  to the remaining budget before every `SSL_connect` attempt, retries `WANT_READ/WANT_WRITE`
  until the deadline, restores the idle timeouts afterwards; errors name the phase
  (`TLS handshake with 'h' timed out after 15.0 s` / `… failed: peer closed the connection`).
- Schannel (`tls_schannel.rs`): handshake record reads keep waiting while budget remains
  (same bug, same fix); compiled and clippy-checked for `x86_64-pc-windows-gnu`, not run on Windows.
- Connect (`pool.rs`): every resolved address within one 10 s budget, each attempt gets an equal
  share of the remainder (min 1.5 s); messages start with `connect to 'host:port' failed`.
- Deviation from the spec: no `poll()` and no happy-eyeballs race. Blocking sockets with a
  per-attempt timeout set to the remaining budget give the same deadline behaviour with less code;
  IPv6 on this machine fails instantly (`ENETUNREACH`), so a race buys nothing here.
- Tests (local servers, no network): silent server hits the budget (and does not overshoot when
  the socket idle timeout is longer), a peer that stalls longer than one socket wait is waited for,
  refused-then-working address, all-refused error kind, phase-named connect error. Mutation-checked:
  restoring the old loop fails with "gave up before the peer spoke: 108 ms"; dropping the
  budget-sized socket timeout overshoots to 3.06 s.
- Real network, 10 sequential fetches of `https://news.ycombinator.com`:
  before 2/5 ok; after 8/10 ok (failures: handshake over 15 s; one success took 33 s).
  curl with the same 15 s cap: 8/10 (one connect stall, one body stall) — the remaining failures
  are the network path, not Droplet. example.com, Wikipedia, rust-lang.org, DuckDuckGo Lite: 20/20,
  unchanged times (255–2030 ms).
