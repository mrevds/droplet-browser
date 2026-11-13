# DROP-123 — Selector coverage

Phase 2 · Status: done · Blocked by: — · Owner: Claude Opus 5.5 (claude-opus-5-5)

## Problem
Corpus `12-selectors`: `:nth-child`, `:first-child`, `[data-x]`, `:not()` match nothing.

## Design
- Attribute selectors: `[a]`, `[a=v]`, `~=`, `|=`, `^=`, `$=`, `*=`, `i` flag.
- Structural: `:first-child`, `:last-child`, `:only-child`, `:nth-child(An+B [of S]?)`,
  `:nth-last-child`, `:*-of-type`, `:empty`, `:root`.
- Logical: `:not(list)`, `:is(list)`, `:where(list)` (zero specificity).
- Combinators `+` and `~` (descendant and `>` exist — verify).
- Specificity per Selectors Level 4. Unsupported selector → drop only that rule, not the sheet.
- Sibling-position lookups must not be O(n²) on long lists: cache child index per element during
  one style pass.

## Acceptance
- Table-driven matching tests incl. Cyrillic attribute values and class names.
- Specificity tests for `:is`/`:not`/`:where`.
- Corpus `12-selectors` → `ok`; works with DROP-120 index if it landed first (both orders must be fine).

## Outcome (Nov 2026)
- All of the design above landed, including `:nth-child(An+B of S)`, `:is`/`:where` (forgiving)
  and `:not` (strict), nesting capped at 8 levels.
- `RuleIndex`: Bloom ancestor keys skip compounds left of `+`/`~`; new buckets for rare attribute
  names, `:root`, and every argument of `:is()`/`:where()`.
- `sibling_index.rs`: positions filled per parent on first query (a whole-document table cost
  ~0.9 ms on Wikipedia).
- Wikipedia style 8.7 → ~10.3 ms: entirely the ~48 rules that used to be dropped and now match
  (with the new syntax disabled the new code measures the same as before). Still under `< 20 ms`.
- `~` scans earlier siblings until a match (O(siblings) per element in the worst case).
