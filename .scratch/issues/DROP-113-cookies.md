# DROP-113 — Cookies

Phase 1 · Status: ready · Blocked by: — · Owner: _(fill in)_

## Problem
No `Cookie` / `Set-Cookie` support: no site login, no consent memory, some sites redirect-loop.

## Design
- `droplet-net`: `CookieJar` (RFC 6265 subset): `Domain` (host-only vs domain match, reject public
  suffix-ish single-label domains), `Path`, `Expires`/`Max-Age`, `Secure` (https only),
  `HttpOnly` (stored; no JS access since JS is off), `SameSite` parsed and stored.
- Hard caps: 50 cookies per domain, 3000 total, 4 KiB per cookie; LRU eviction.
- Session cookies in memory; persistent cookies saved through `droplet-storage` next to history
  (file format documented in ARCHITECTURE.md). Redirect responses also set cookies.
- `about:cookies` page or a line in `about:history` is optional, not required.

## Acceptance
- Unit tests: domain/path matching table, expiry, `Secure` not sent over http, caps and eviction,
  Cyrillic/IDN host stored in punycode-or-lowercase consistently, malformed headers ignored.
- Local server test: `Set-Cookie` on a 302, cookie sent on the follow-up request.
- Persistence test with a temp data dir.
