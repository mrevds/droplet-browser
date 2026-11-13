# DROP-005: Forms, Focus, Selection & Clipboard

- **Phase:** 5
- **Status:** `in-progress`
- **Blocked By:** `DROP-002` (`done`)
- **Implementation owner:** DeepSeek V4.1 Flash Free (OpenCode agent)

---

## Goal

Make Droplet pages interactive: real HTML form controls, keyboard focus traversal,
a visible focus ring, mouse text selection, and clipboard copy/cut/paste — all in
pure Rust `std` with zero new crate dependencies and no regressions to the
`Copy` `ComputedStyle` / Arena DOM invariants.

## Required scope

1. **Controls** — `<input type="text|password|checkbox|radio|submit">`,
   `<textarea>`, `<button>`, `<select>/<option>`; `name`, `value`, `placeholder`,
   `checked`, `selected`, `disabled`, `action`, `method` attributes.
2. **Layout & paint** — controls are replaced/atomic boxes with sensible UA
   defaults (width, height, border, padding). Text inputs/textarea draw their
   value, caret and placeholder; password inputs draw bullets; checkboxes/radios
   draw checked state; selects draw the selected option; buttons draw their label.
3. **Focus** — click focuses a control, `Tab`/`Shift+Tab` move focus, focus ring is
   painted, `:focus` can be styled, `Esc` blurs. Focus state is per-tab.
4. **Editing** — printable UTF-8 input, backspace, delete, Home/End, arrow keys
   inside text fields/textarea; checkbox/radio toggle; select cycles options.
5. **Selection** — mouse drag selection over rendered text, `Ctrl+A` select all,
   `Ctrl+C`/`Ctrl+X`/`Ctrl+V` via platform clipboard, selection highlight overlay.
6. **Forms** — submit via submit button / Enter; URL-encoded `GET` (query string)
   and `POST` (body); named controls; UTF-8 percent encoding.
7. **Tests** — unit/regression tests for control parsing, editing (including
   Cyrillic), password masking, checkbox/radio groups, URL encoding, focus
   traversal, selection ranges, clipboard round-trip helpers, and one headless
   end-to-end form submit through a local `TcpListener`.

## Constraints

- `Cargo.toml [dependencies]` stays empty in every crate.
- `ComputedStyle` remains `Copy`; do not add `String`/`Vec`/`HashMap` fields.
- DOM traversal stays iterative; attributes stay `Vec<(String, String)>`.
- Every parser must be UTF-8 boundary safe.
- Keep changes scoped to forms/focus/selection/clipboard; do not implement
  find-in-page, history, or JavaScript.
