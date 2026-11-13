# DROP-124 — `box-sizing`, `min/max-*`, shorthands

Phase 2 · Status: done · Blocked by: — · Owner: Claude Opus 5.5 (claude-opus-5-5)

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

## Outcome (2026-11-13)
- Style: `ComputedStyle` gained `box_sizing: BoxSizing`, `min_width`/`min_height`/`max_height`
  and `inset: InsetSizes` as `LengthPct` (`Auto | Px | Percent`); `width`/`max_width`/`height`
  moved from `Option<f32>` + `*_pct` pairs to `LengthPct`. Size 148 → 184 bytes (still `Copy`).
- Parsing lives in `crates/droplet-style/src/shorthand.rs`; every shorthand is all-or-nothing.
  `border*` now follows CSS: a side without a visible `border-style` has width 0 (so
  `border: 1px #000` or a lone `border-width` draws nothing). `background` without a color resets
  it; a gradient falls back to its first stop. `font` sets size (also in the em pass), weight and
  monospace family; line-height is parsed and ignored (no line-height model yet).
  `box-sizing: inherit` is supported (the `*{box-sizing:inherit}` idiom). Until DROP-121,
  `var(--x, fallback)` inside these shorthands uses its fallback.
- Layout: `box_model.rs` `resolve_width/height(_pb)` return the used content size (box-sizing,
  then max, then min — min wins); `clamp_width/height(_pb)` clamp auto sizes. Block, inline-block,
  forms (with effective control padding), images (aspect kept) and flex (CSS §9.7 freeze loop
  for min/max, border-box `flex-basis`) use them.
- Not done (follow-ups): percentage padding/margins; per-side border colors; `min-width: auto`
  for flex items is 0 (not min-content); `height` still grows to fit content (overflow, DROP-133);
  `inset` is stored only (DROP-130).
- Bench: see the commit message. Corpus `09-box-sizing` → `ok`.
