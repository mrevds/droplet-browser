# DROP-112 — `Content-Encoding: gzip / deflate`

Phase 1 · Status: done · Blocked by: — · Owner: Claude Opus 5.5 (Claude Code)

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

## Result (2026-11-13)
- New crate `crates/droplet-inflate` (no dependencies): the PNG inflate moved here
  (`git mv` from `droplet-image/src/deflate.rs`), plus `inflate_raw[_into]`, `gunzip`,
  `looks_like_zlib`, table-driven `crc32`, own `InflateError`. `droplet-image` and `droplet-net`
  both depend on it; graph stays acyclic.
- `droplet-net/src/pool.rs`: `Accept-Encoding: gzip, deflate`; `Content-Encoding` tokens parsed
  (several headers / comma lists), decoded right to left after de-chunking in both the streaming
  path and `parse_http_response`; `deflate` accepts zlib or raw; unknown → load error.
- Bomb guard: decoded size capped at 16 MiB *during* inflation (fixed-Huffman zero bomb of
  ~130 KB that would expand to ~21 MB fails with `decompressed data exceeds size limit`).
- Tests: CPython-generated vectors (empty, ASCII, Cyrillic, two members, FNAME, zlib, raw),
  CRC-32 reference values, corrupt CRC / ISIZE / truncation / reserved flags / trailing garbage,
  NUL padding, bomb (raw and gzip); HTTP: chunked+gzip round trip and `Accept-Encoding` sent,
  raw deflate with Content-Length, corrupt gzip and `br` reported as errors, identity, 304.
- Real network, old (`main`) vs new binary, `--dump-dom`, 5 alternating runs each, DOM identical:

  | site | bytes identity → gzip | old ms | new ms |
  | --- | --- | --- | --- |
  | Wikipedia Rust article | 1 065 646 → 159 587 | 920 937 1218 1229 2507 | 668 659 679 901 1330 |
  | example.com | 577 → 400 | 235–504 | 236–459 |
  | rust-lang.org, DDG Lite | not compressed by server | unchanged | unchanged |

- Inflate cost: 5.6 ms for 1 MB (C zlib 2.4 ms). Table-driven Huffman decoding is a Phase 5
  candidate. Binary +13 KB (2 432 024 → 2 445 352 bytes).
