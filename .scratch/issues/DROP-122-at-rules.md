# DROP-122 — `@media`, `@import`, `@supports`

Phase 2 · Status: ready · Blocked by: — · Owner: _(fill in)_

## Problem
At-rules are skipped wholesale: `@media (min-width: 800px) { … }` never applies (corpus `07-media`),
so responsive sites show their mobile/hidden layers; `<link media=…>` is ignored.

## Design
- Parse media query lists: media types `all`/`screen`/`print`, `not`/`only`, features
  `min-/max-width`, `min-/max-height`, `width`/`height`, `orientation`, `prefers-color-scheme`
  (light), `prefers-reduced-motion` (reduce), `hover`/`pointer` (fine). Unknown feature → false.
- Rules keep their media condition; evaluated with `StyleContext` viewport so resize re-evaluates.
- `@import url(...) [media]` fetched with the stylesheet batch, depth ≤ 4, cycle-safe, counts
  toward `MAX_STYLESHEETS_PER_PAGE`. `<link rel=stylesheet media=print>` not applied on screen.
- `@supports`: evaluate `(property: value)` against what the parser understands; `not/and/or`.

## Acceptance
- Parser tests per feature, malformed queries, nested `@media` inside `@supports`.
- Resize test: crossing 800 px flips which block is displayed.
- Corpus `07-media` → `ok`; Wikipedia screenshot no longer shows hidden menus expanded (before/after images in the PR).
