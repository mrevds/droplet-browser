# DROP-003: HTTPS (TLS), Keep-Alive Pool, Concurrent CSS Loader & Tracker Filter

- **Phase:** Phase 3 (Ready)
- **Status:** `ready-for-agent`
- **Blocked By:** `DROP-001` (`done`)

---

## Goal

Enable real-world `https://` browsing with a minimal, zero-bloat TLS stack, reuse TCP/TLS connections via HTTP `Keep-Alive` pooling, fetch external `<link rel="stylesheet">` files concurrently off the UI thread, cache DNS lookups in memory, and block known tracker/ad domains before any socket connection is opened.

---

## Files to Edit

1. **`Cargo.toml`**
   - Keep compile time and binary size minimal. If adding a TLS dependency (e.g., `rustls` + `webpki-roots` / `rustls-native-certs`), disable unused features (`default-features = false`, minimal crypto provider) and verify `cargo build` stays fast with no async runtime (`tokio`/`async-std` forbidden).

2. **`src/url.rs`**
   - Add `Scheme::Https` alongside `Http`, `File`, `About`.
   - Parse `https://host[:port][/path][?query][#fragment]` with default port `443`.
   - Remove the Phase 1 `https://` rejection error in `Url::parse` and `Url::resolve`.
   - Resolve protocol-relative `//host/path` from an `Https` base URL to `https://host/path`.

3. **`src/net.rs` (and optional `src/adblock.rs`)**
   - **Tracker / Ad Filter:** Zero-overhead domain matcher (`TrackerFilter`) checking exact domain and parent domain suffixes before DNS/TCP connect.
   - **DNS Cache:** In-memory `(host, port) -> (Vec<SocketAddr>, Instant)` cache with configurable TTL (e.g. 60s).
   - **Connection Pool & Keep-Alive:** Pool idle `TcpStream` / TLS streams keyed by `(Scheme, host, port)`. Implement `Content-Length` and incremental `chunked` stream reading so connections return to the pool without waiting for server EOF.
   - **TLS Transport:** Wrap `TcpStream` in TLS using system root CAs (`/etc/ssl/certs`) and SNI hostname verification.
   - **Concurrent Resource Loader:** `fetch_many(urls: &[Url]) -> Vec<Result<Response, NetError>>` backed by a bounded `std::thread` pool (max 4 workers).

4. **`src/dom.rs` & `src/browser.rs`**
   - Add `Document::collect_stylesheet_links(&self) -> Vec<String>` (extracting `<link rel="stylesheet" href="...">` in document order).
   - Resolve `<link>` URLs relative to the page URL, fetch them concurrently via `fetch_many`, and apply external stylesheets in HTML document order together with inline `<style>` rules.

---

## Edge Cases to Handle

- **Stale Pooled Connections:** If a pooled `Keep-Alive` socket was closed by the remote server (`BrokenPipe` / `UnexpectedEof` before reading status line), transparently discard it and retry once on a fresh connection.
- **HTTPS Redirects & Mixed Redirects:** Follow `301`/`302`/`303`/`307`/`308` across `http://` <-> `https://` (max 5 hops) with accurate relative URL resolution.
- **Invalid Certificates:** Self-signed, expired, or hostname-mismatched TLS certificates must return a clear `NetError`—never bypass verification.
- **Non-Fatal Subresource Failure:** If an external `<link rel="stylesheet">` times out or returns 404/500, skip that stylesheet cleanly and keep rendering the page.
- **Domain Suffix Boundary Safety:** Blocking `tracker.example` must block `ads.tracker.example` and `tracker.example`, **not** `mytracker.example`.
- **Keep-Alive Chunked Framing:** Correctly detect end of chunked stream (`0\r\n\r\n`) on the socket without over-reading into a future response or hanging until timeout.

---

## Acceptance Tests

1. **URL Tests (`src/url.rs`):**
   - `Url::parse("https://example.com/a?x=1")` -> `Scheme::Https`, `port == 443`, `path == "/a"`.
   - `https_base.resolve("//cdn.example.com/app.css")` -> `"https://cdn.example.com/app.css"`.
2. **Tracker Filter Tests (`src/net.rs` / `src/adblock.rs`):**
   - Exact domain and subdomain match -> blocked (`NetError` before network).
   - Unrelated suffix collision (`notdoubleclick.net`) -> allowed.
3. **Loopback Keep-Alive & Framing Tests (`src/net.rs`):**
   - Local `TcpListener` serving two sequential HTTP/1.1 `Keep-Alive` responses (`Content-Length` + `Transfer-Encoding: chunked`) over a single reused TCP connection.
   - Stale pooled socket EOF triggers clean one-shot reconnect.
   - `fetch_many` fetches multiple local endpoints concurrently.
4. **Cascade Integration Test (`src/browser.rs`):**
   - Page with `<link rel="stylesheet" href="...">` + inline `<style>` applies styles in document order and survives missing stylesheet gracefully.
5. **Global Gate:**
   - `/home/mrevds/.cargo/bin/cargo test`
   - `/home/mrevds/.cargo/bin/cargo clippy --all-targets`
