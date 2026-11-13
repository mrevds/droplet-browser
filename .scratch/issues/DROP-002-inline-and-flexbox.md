# DROP-002: Full Inline-Block, Flexbox Layout, Relative Units & Pseudo-Classes

- **Phase:** Phase 2 (Ready — Next)
- **Status:** `ready-for-agent`
- **Blocked By:** `DROP-001` (`done`)

---

## Goal

Upgrade Droplet's CSS cascade and layout engine from basic block + inline text flow to modern responsive layout primitives (`display: inline-block` and `display: flex`), full relative unit resolution (`%`, `em`, `rem`, `vh`, `vw`), and interactive `:hover` / `:active` pseudo-classes without sacrificing sub-2ms layout+paint speed.

---

## Files to Edit

1. **`src/css.rs`**
   - Extend `Unit` with `Vh` and `Vw` (parse `vh`, `vw` length suffixes alongside `px`, `%`, `em`, `rem`).
   - Extend `CompoundSelector` with pseudo-class flags (`hover: bool`, `active: bool`) and parse `:hover` / `:active` (counting each pseudo-class in specificity class bucket `(0, 1, 0)` per CSS Selectors spec).
   - Parse Flexbox keywords/values (`flex`, `inline-block`, `row`, `column`, `row-reverse`, `column-reverse`, `wrap`, `nowrap`, `wrap-reverse`, `flex-start`, `flex-end`, `center`, `space-between`, `space-around`, `space-evenly`, `stretch`, `baseline`).

2. **`src/style.rs`**
   - Extend `Display` enum: `Block`, `Inline`, `InlineBlock`, `Flex`, `None`.
   - Add Flexbox & sizing fields on `ComputedStyle`:
     - `flex_direction`, `flex_wrap`, `justify_content`, `align_items`, `gap`
     - `flex_grow`, `flex_shrink`, `flex_basis`
   - Pass `StyleContext { viewport_width: f32, viewport_height: f32, root_font_size: f32, hovered_node: Option<NodeId>, active_node: Option<NodeId> }` (and ancestor-aware hover/active matching so hovering a child text/span matches `a:hover`).
   - Resolve `em`, `rem`, `vh`, `vw`, and `%` lengths accurately against parent/own `font_size`, root `font_size`, viewport, or containing block dimensions.

3. **`src/layout.rs`**
   - **`inline-block` formatting:** Lay out `Display::InlineBlock` children as shrink-to-fit atomic inline boxes placed on line boxes alongside text runs, wrapping cleanly to the next line when `cursor_x + box_width > available_width` and aligning vertically within the line box.
   - **Flexbox layout (`Display::Flex`):** Two-pass flex container algorithm:
     - Determine main/cross axes from `flex-direction`.
     - Compute hypothetical base sizes (`flex-basis` -> explicit `width`/`height` -> shrink-to-fit content).
     - Split into flex lines when `flex-wrap: wrap` / `wrap-reverse` (respecting `gap`).
     - Distribute positive free space via `flex-grow` and shrink overflow via `flex-shrink` (clamped at `>= 0.0`).
     - Position items along main axis (`justify-content`) and cross axis (`align-items`: `stretch`, `flex-start`, `flex-end`, `center`).

4. **`src/browser.rs` & `src/window.rs`**
   - Track pointer coordinates `(mouse_x, mouse_y)` and pressed state in `Browser`.
   - Hit-test `hovered_node` on mouse move and `active_node` on mouse press/release.
   - Trigger dirty restyle + relayout + repaint **only** when `hovered_node` or `active_node` actually changes and the stylesheet contains `:hover` or `:active` rules.

---

## Edge Cases to Handle

- **Nested Flex Containers:** A flex item that itself has `display: flex` must compute its intrinsic/content size in pass 1 without double-applying parent margins.
- **Negative Free Space:** `flex-shrink` must never produce negative item widths/heights; clamp minimum main-axis size to `padding + border` (or `0.0`).
- **`em` Self vs Parent:** `font-size: 2em` resolves relative to the **parent** element's `font_size`; `padding: 1.5em` / `width: 10em` resolves relative to the element's **own** computed `font_size`. `rem` always resolves against `<html>` root `font_size`.
- **Percentage Height Fallback:** When parent height is `auto` (unconstrained), percentage `height` treats `auto` safely instead of propagating `NaN` or `0` unexpectedly.
- **Ancestor `:hover` Propagation:** Hovering text inside `<a href="..."><span>Link</span></a>` must match `a:hover`, not just the inner `<span>`.
- **Zero Overhead When Idle:** Moving the mouse over a page with no `:hover` rules must do zero restyle/relayout work.

---

## Acceptance Tests

1. **CSS Unit & Selector Tests (`src/css.rs`):**
   - `a:hover`, `.btn:active:hover` parse with specificity `(0, 1, 1)` and `(0, 3, 0)` respectively.
   - `50vh`, `25vw`, `1.5rem`, `2em`, `50%` parse into exact `Value::Length` variants.
2. **Style Resolution Tests (`src/style.rs`):**
   - Nested `em` inheritance (`body { font-size: 20px }` -> `div { font-size: 1.5em; padding: 1em; }` yields `font_size == 30.0`, `padding.left == 30.0`) while `rem` stays relative to root `<html>`.
   - `:hover` and `:active` match when `hovered_node` / `active_node` points to the element or its descendant.
3. **Layout Tests (`src/layout.rs`):**
   - Two `inline-block` boxes fit side-by-side when width allows, and wrap to a second line when container narrows.
   - `display: flex` row with `justify-content: space-between`, `align-items: center`, `gap: 12px`, and `flex-grow: 1` places items at exact coordinates.
   - `flex-wrap: wrap` wraps overflowing items onto multiple flex lines without overlap.
   - `flex-direction: column` stacks flex items vertically with `gap`.
4. **End-to-End Verification:**
   - `/home/mrevds/.cargo/bin/cargo test`
   - `/home/mrevds/.cargo/bin/cargo clippy --all-targets`
   - `/home/mrevds/.cargo/bin/cargo run -- --dump-layout about:test`
