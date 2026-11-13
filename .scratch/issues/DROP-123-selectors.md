# DROP-123 — Selector coverage

Phase 2 · Status: ready · Blocked by: — · Owner: _(fill in)_

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
