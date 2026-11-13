# DROP-112 — `Content-Encoding: gzip / deflate`

Phase 1 · Status: ready · Blocked by: — · Owner: _(fill in)_

## Problem
`crates/droplet-net/src/pool.rs` sends `Accept-Encoding: identity`. Wikipedia's article HTML is
1 MB uncompressed (~200 KB gzipped); every page pays 3–5× the bytes.

## Design
- Move the inflate implementation used by the PNG decoder (`crates/droplet-image/src/png.rs`)
  into a small shared module (new crate `droplet-inflate` or a `pub mod inflate` in
  `droplet-image` re-used by `droplet-net` — pick the one that keeps the dependency graph acyclic
  and document it in ARCHITECTURE.md).
- gzip wrapper (RFC 1952 header, CRC32 + ISIZE check) and zlib/raw deflate for `deflate`.
- Send `Accept-Encoding: gzip, deflate`; decode after de-chunking; enforce the 16 MiB limit on
  the **decoded** size (zip-bomb guard) and abort early when exceeded.

## Acceptance
- Unit tests: known gzip vectors (empty, short ASCII, Cyrillic UTF-8, multi-member), corrupted CRC
  → error, decoded size > 16 MiB → error without allocating it all.
- Local server test: chunked + gzip response round-trips.
- `--bench` Wikipedia `fetch_ms` before/after in the commit message.
