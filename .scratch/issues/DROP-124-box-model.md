# DROP-124 — `box-sizing`, `min/max-*`, shorthands

Phase 2 · Status: ready · Blocked by: — · Owner: _(fill in)_

## Problem
Corpus `09-box-sizing`: `box-sizing: border-box; width: 100%; padding: 20px` overflows its parent
by 40 px. Same bug makes rust-lang.org's "Get Started" bar run off the right edge.

## Design
- `box-sizing: content-box | border-box` in `ComputedStyle` (stays `Copy`), applied in block,
  inline-block and flex sizing.
- `min-width`, `max-width`, `min-height`, `max-height` (lengths, `%`, `none`) with the CSS 2.1
  clamping order; flex items respect them.
- Shorthand audit with tests: `margin`, `padding` (1–4 values), `border`, `border-<side>`,
  `border-width/style/color`, `background` (color only is fine), `font` (size/weight/family,
  optional line-height), `inset` (store for DROP-130).

## Acceptance
- Layout tests: border-box vs content-box widths, percentages, min > max precedence.
- Shorthand parsing tests incl. invalid combos ignored.
- Corpus `09-box-sizing` → `ok`; rust-lang.org before/after screenshot in the PR.
