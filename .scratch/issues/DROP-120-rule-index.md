# DROP-120 — Selector rule index

Phase 2 · Status: done · Blocked by: — · Owner: Claude Opus 5.5 (Claude Code)

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

## Result (2026-11-13)
- `crates/droplet-style/src/rule_index.rs`: `RuleIndex` (id / first class / tag / universal buckets
  of the rightmost compound, built per style pass; UA sheet index cached in a `OnceLock`) and
  `AncestorBloom` (4 KiB counting Bloom filter of ancestor tag/id/classes, maintained by the
  depth-first walk in `build_styled_node`, sticky on saturation so it never gives false negatives).
- Equivalence: randomized test (40 seeds, Cyrillic ids/classes, `>`/descendant, `:hover`/`:active`/
  `:focus`/unsupported pseudo) compares every element's matched rules with the brute-force path;
  mutation-checked (dropping the universal bucket or a broken Bloom fails it). Full `--dump-layout`
  of the Wikipedia article (10 696 lines) is byte-identical to the pre-change binary.
- Bench, offline copy of the Wikipedia Rust article (19 424 nodes, 248 KB CSS, median of 10):

  | | before | after |
  | --- | --- | --- |
  | style_ms | 265.4 | 8.5 |
  | cpu_ms | 299.9 | 41.8 |
  | peak RSS | 54.3 MB | 54.7 MB |
  | `about:droplet` cpu_ms | 1.49 | 1.46 |

- Observed while profiling (not fixed here): the CSS parser keeps only 479 of ~2 400 selectors on
  that page because `@media` blocks are dropped (`DROP-122`); `build_styled_node` is still
  recursive, against the iterative-traversal invariant (candidate for Phase 5).
