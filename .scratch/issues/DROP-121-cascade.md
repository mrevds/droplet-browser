# DROP-121 — Cascade correctness: `!important`, CSS-wide keywords, `var()`

Phase 2 · Status: ready · Blocked by: — · Owner: _(fill in)_

## Problem
`crates/droplet-style/src/css.rs` drops `!important`; `inherit`/`initial`/`unset` are partial;
custom properties and `var()` are unsupported (corpus `08-cascade`: `#a` red, `#c` has no height).

## Design
- Declarations carry an `important` bit; cascade order: normal author < important author
  (inline `style` participates in both levels).
- `inherit` / `initial` / `unset` / `revert` (treat as `unset`) for every supported property.
- Custom properties: stored **outside** `ComputedStyle` (it must stay `Copy`): a per-node side
  table `Vec<(Atom/String, String)>` only for elements that declare or inherit any `--*`,
  shared by index when unchanged from the parent. Resolve `var(--x, fallback)` at computed-value
  time, detect cycles (→ invalid at computed time), cap substitution depth/length.

## Acceptance
- Unit tests for each keyword and for importance ordering incl. inline style.
- `var()` tests: fallback, nested fallback, cycle, inheritance through 3 levels, Cyrillic values.
- Corpus `08-cascade` reviewed → `ok`; goldens re-blessed; bench not worse.
