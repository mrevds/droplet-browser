# DROP-120 — Selector rule index

Phase 2 · Status: ready · Blocked by: — · Owner: _(fill in)_

## Problem
`droplet --bench` on the Wikipedia Rust article: `style_ms ≈ 261` of `cpu_ms ≈ 295`
(19 424 DOM nodes, 248 KB CSS). The cascade (`crates/droplet-style/src/cascade.rs`) tests every
rule against every element.

## Design
- Build once per stylesheet: buckets keyed by the **rightmost compound** selector's id, else one
  class, else tag, else a universal list (like Servo/Blink rule hashes). Store indices into the
  rule vector, keep source order for the cascade.
- Per element: gather candidates from its id, each class, tag and universal bucket; merge by
  (specificity, source order); then run full right-to-left matching only on candidates.
- Optional cheap filter: a 64-bit ancestor bloom (ids/classes/tags of ancestors) to reject
  descendant selectors early. Must stay allocation-free per node in the common case.
- `ComputedStyle` stays `Copy`; the index lives beside `Stylesheet`, not inside styles.

## Acceptance
- Existing cascade tests pass unchanged; add tests proving identical results with and without
  the index on a randomised selector/document set (deterministic seed, Cyrillic class names).
- Corpus goldens unchanged.
- Wikipedia `style_ms < 20` (paste `--bench` before/after); `about:droplet` not worse.
