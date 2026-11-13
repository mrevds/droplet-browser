# Droplet Execution Frontier (`SPEC.md`)

Source of truth for picking work. Phases and rules: [ROADMAP.md](../ROADMAP.md).
Old tickets `DROP-001..011` are archived in [archive/](archive/) — their `done` status meant
"tests + clippy green", not "renders correctly", so they are history only.

## How to take a ticket

1. Read [CLAUDE.md](../CLAUDE.md), [ROADMAP.md](../ROADMAP.md) (rules section), [ARCHITECTURE.md](../ARCHITECTURE.md).
2. Pick the **lowest-numbered** `ready` ticket in the **earliest unfinished phase**. Do not start a
   later phase while an earlier one has `ready` tickets. Spec lives in `issues/DROP-*.md`;
   if the spec column says `draft`, write the spec first and get it reviewed.
3. Work on a branch, one ticket per branch. Record bench numbers **before** you change code:
   `cargo run --release -- --bench <page>` for the pages listed in the ticket.
4. Done gate (all required):
   - acceptance tests in the ticket pass; `cargo test --workspace`; `cargo clippy --workspace --all-targets`;
   - corpus pages in the ticket's scope reviewed from a screenshot and flipped to `ok` in
     `tests/corpus/manifest.txt`; goldens re-blessed (`DROPLET_BLESS=1 cargo test -p droplet-desktop --test corpus`)
     and the diff read;
   - bench numbers after the change are not worse (paste before/after in the commit message);
   - status flipped here, downstream `blocked` → `ready` when all blockers are `done`.

Status values: `done`, `ready`, `blocked`, `frozen` (do not touch until its phase).

## Frontier

| ID | Phase | Title | Status | Blocked by | Corpus / pages | Spec |
| --- | --- | --- | --- | --- | --- | --- |
| `DROP-100` | P0 | Hygiene & measurement: remove `src/`, JS off by default, `sandbox` feature, unified `--dump-layout`, `--bench`, corpus goldens | `done` | — | all (baseline) | this file |
| `DROP-110` | P1 | Navigation off the UI thread: worker + channel, cancel, progressive status | `done` | — | HN, Wikipedia (GUI responsiveness) | [spec](issues/DROP-110-async-navigation.md) |
| `DROP-111` | P1 | Reliable TLS handshake & timeouts (poll-based, separate connect/handshake/read deadlines) | `done` | — | HN | [spec](issues/DROP-111-tls-timeouts.md) |
| `DROP-112` | P1 | `Content-Encoding: gzip/deflate` (reuse PNG inflate) | `done` | — | Wikipedia | [spec](issues/DROP-112-gzip.md) |
| `DROP-113` | P1 | Cookies (RFC 6265 subset, bounded jar, `Secure`/`HttpOnly`/`Path`/`Expires`) | `ready` | — | login flows | [spec](issues/DROP-113-cookies.md) |
| `DROP-120` | P2 | Selector rule index (buckets by id/class/tag, right-to-left match) | `done` | — | Wikipedia style `261 ms → < 20 ms` | [spec](issues/DROP-120-rule-index.md) |
| `DROP-121` | P2 | Cascade: `!important`, `inherit`/`initial`/`unset`, custom properties + `var()` | `ready` | — | `08-cascade` | [spec](issues/DROP-121-cascade.md) |
| `DROP-122` | P2 | `@media` (width/height/orientation/`prefers-color-scheme`/print), `@import`, `@supports` basic | `ready` | — | `07-media`, Wikipedia | [spec](issues/DROP-122-at-rules.md) |
| `DROP-123` | P2 | Selectors: `:not`, `:nth-child/-of-type`, `:first/last-child`, attribute operators, `+` `~` | `ready` | — | `12-selectors` | [spec](issues/DROP-123-selectors.md) |
| `DROP-124` | P2 | `box-sizing`, `min/max-width/height`, shorthand audit (margin/padding/border/background/font) | `ready` | — | `09-box-sizing`, rust-lang.org | [spec](issues/DROP-124-box-model.md) |
| `DROP-130` | P3 | Positioned layout: relative/absolute/fixed/sticky + `z-index` stacking | `blocked` | `DROP-124` | `05-position` | draft |
| `DROP-131` | P3 | Floats & `clear` | `blocked` | `DROP-124` | `06-float` | draft |
| `DROP-132` | P3 | Table layout (auto widths, `colspan`/`rowspan`, `border-collapse`) | `blocked` | `DROP-124` | `04-table`, HN | draft |
| `DROP-133` | P3 | `overflow` clip/scroll containers, `visibility` | `blocked` | `DROP-130` | `11-overflow` | draft |
| `DROP-134` | P3 | Margin collapsing, line breaking (UAX #14 subset), `white-space` | `blocked` | `DROP-124` | `01`, Wikipedia | draft |
| `DROP-140` | P4 | Italic/oblique + font fallback chains, kerning | `blocked` | Phase 3 | `02-inline` | draft |
| `DROP-141` | P4 | WebP decoder (lossless + lossy) | `blocked` | Phase 3 | real sites | draft |
| `DROP-142` | P4 | Image memory budget, lazy decode outside viewport | `blocked` | Phase 3 | Wikipedia RSS `< 40 MB` | draft |
| `DROP-150` | P5 | Scroll without relayout (shift cached frame, repaint exposed strip) | `blocked` | Phase 4 | — | draft |
| `DROP-151` | P5 | Incremental restyle/relayout of dirty subtrees | `blocked` | Phase 4 | — | draft |
| `DROP-152` | P5 | Enforced perf budgets: bench regression test over corpus | `blocked` | Phase 4 | all | draft |
| `DROP-153` | P5 | Idle CPU + multi-tab RAM measurement; verify hibernation frees memory | `blocked` | Phase 4 | — | draft |
| `DROP-160` | P6 | Forms complete (encodings, checkbox/radio, `select multiple`) | `blocked` | Phase 5 | — | draft |
| `DROP-161` | P6 | Downloads | `blocked` | Phase 5 | — | draft |
| `DROP-162` | P6 | Session restore | `blocked` | Phase 5 | — | draft |
| `DROP-170` | P7 | ADR: hybrid engine — own engine for documents, WebKitGTK (`dlopen`, out-of-process `droplet-webview`, XEmbed) for script-heavy sites; prototype | `done` | — | YouTube (screenshot + PSS) | [ADR](issues/DROP-170-hybrid-engine.md) |
| `DROP-171` | P7 | Hybrid UX: click in WebKit view unfocuses address bar, autocomplete not hidden behind view, WebKit downloads/new-window/`target=_blank` → Droplet tabs | `ready` | — | YouTube, Gmail | draft |
| `DROP-172` | P7 | Ad/tracker blocking in WebKit tabs via WebKit content filters (reuse `TrackerFilter` lists) | `ready` | — | YouTube, news sites | draft |
| `DROP-173` | P7 | Hibernation policy for WebKit tabs: never freeze a tab playing audio; memory pressure → freeze oldest | `ready` | — | YouTube music in background | draft |
| `DROP-174` | P7 | Engine switch UI: badge in address bar, per-site toggle written to `webkit-sites` | `ready` | — | — | draft |
| `DROP-175` | P7 | Codec check: detect missing GStreamer elements, show a one-line hint (`apt install gstreamer1.0-plugins-bad gstreamer1.0-libav`) | `ready` | — | YouTube video | draft |
| `DROP-180` | P8 | Sandbox wired into the GUI process | `frozen` | Phase 6 | — | draft |
| `DROP-181` | P8 | Parser fuzzing harness (HTML/CSS/PNG/JPEG/TLS records), no deps | `frozen` | Phase 6 | — | draft |
| `DROP-182` | P8 | `unsafe` audit (~400 sites) | `frozen` | Phase 6 | — | draft |
| `DROP-190` | P9 | Windows parity on corpus | `frozen` | Phase 6 | all | draft |
| `DROP-191` | P9 | Wayland parity with X11 | `frozen` | Phase 6 | all | draft |
| `DROP-192` | P9 | macOS (needs a macOS host) | `frozen` | Phase 6 | all | draft |

Phase 7 (hybrid engine) was pulled forward by DROP-170: the user's goal is a full Firefox
replacement, which is impossible without it. P7 tickets touch `droplet-webview`, `webview.rs`
and the X11 embedding only, so they may run in parallel with P1–P3 work.

Phase 1 and Phase 2 tickets are independent of each other and may run in parallel —
**each on its own branch/worktree**, never in one shared dirty checkout.

## Frozen code

No feature work until its phase: `crates/droplet-js` (superseded by the hybrid engine, DROP-170), `crates/droplet-sandbox` and
`crates/droplet-ipc` (P8), `crates/droplet-platform/src/linux_wayland.rs` and `macos.rs` (P9).
Build fixes are allowed.

## Global verification gate

```bash
/home/mrevds/.cargo/bin/cargo test --workspace
/home/mrevds/.cargo/bin/cargo clippy --workspace --all-targets
/home/mrevds/.cargo/bin/cargo clippy -p droplet-desktop --features sandbox --all-targets
/home/mrevds/.cargo/bin/cargo test -p droplet-desktop --test corpus
/home/mrevds/.cargo/bin/cargo run --release -- --bench about:droplet
tools/bench-sites.sh   # network; real-site smoke + numbers
```
