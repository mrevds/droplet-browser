# DROP-008: Reactive DOM Bindings, Event Dispatch & Incremental Dirty Relayout/Repaint

- **Phase:** 7b
- **Status:** `done`
- **Blocked By:** `DROP-005` (`done`), `DROP-007` (`done`)
- **Implementation owner:** Swarm Wave 2 (`js-vm`, `forms`)

---

## Goal

Connect `droplet-js::JsRuntime` to `droplet-dom::Document`, user input events, and `droplet-shell::Browser` so page scripts can query/mutate the live DOM, handle click/input/submit/change/key events, schedule timers (`setTimeout`/`clearTimeout`/`setInterval`/`clearInterval`), and trigger minimal dirty relayout/repaint cycles — with zero new crate dependencies and strict `Copy` `ComputedStyle` / Arena DOM invariants preserved.

## Required scope

1. **JS Host Bridge (`crates/droplet-js` / `crates/droplet-shell`):**
   - Expose host globals and native functions to `JsRuntime` without coupling `droplet-js` to `droplet-shell`: use a lightweight host callback/trait (`DomHost` or host native hooks + shared context) or a clean bridge module.
   - `document`:
     - `document.getElementById(id)`
     - `document.querySelector(selector)` & `document.querySelectorAll(selector)` (matching tag, `#id`, `.class`, descendant/child combinators cleanly iteratively)
     - `document.createElement(tag)` & `document.createTextNode(text)`
     - `document.body`, `document.title` (getter/setter updating tab title)
   - `Element` wrapper objects backed by `NodeId`:
     - `id`, `tagName`, `value` (for form controls), `checked` (checkbox/radio)
     - `textContent` (getter & setter — UTF-8 safe iterative subtree text update)
     - `innerHTML` (getter / constrained setter parsing fragment cleanly into arena)
     - `getAttribute(name)`, `setAttribute(name, val)`, `removeAttribute(name)`, `hasAttribute(name)`
     - `classList` (`add`, `remove`, `toggle`, `contains`)
     - `style` proxy/object allowing inline CSS property reads/writes (e.g. `el.style.color = "red"`, `el.style.backgroundColor`, `el.style.display = "none"` updating the element's `style` attribute cleanly)
     - `appendChild(child)`, `removeChild(child)` (preventing cycles & updating `parent`/`children` in `Document` arena)
     - `addEventListener(type, listenerFn)` & `removeEventListener(type, listenerFn)`
     - `focus()` / `blur()` (syncing with `FormSessionState` focus)
   - `console.log(...)` forwarding into a per-tab bounded console log buffer (viewable in inspector/tests) + `window` aliases where useful.
2. **Event Dispatch & Bubbling (`crates/droplet-shell`):**
   - Synthetic `Event` object passed to JS listeners: `{ type, target, currentTarget, key, value, checked, preventDefault(), stopPropagation() }`.
   - Bubbling propagation from deepest hit `NodeId` up `parent` chain to `Document` root (unless `stopPropagation()` called).
   - Wire DOM events into `Browser`:
     - `click` on mouse down/up or `handle_activate` (`preventDefault()` cancels `<a href>` navigation or form submission / button toggle).
     - `input` and `change` when form controls (`<input>`, `<textarea>`, `<select>`, checkbox/radio) change value or selection.
     - `submit` on form submit (`preventDefault()` cancels navigation).
     - `keydown` / `keyup` when page control focused.
3. **Inline & External `<script>` Execution:**
   - Collect `<script>` tags in document order (supporting inline text and `src="..."` fetched via `droplet-net` respecting `TrackerFilter`).
   - Run scripts after initial HTML parse & CSS load; keep execution budgeted (`MAX_EXECUTION_STEPS` / instruction quota in `JsRuntime`) so runaway scripts never freeze the UI thread, catching/recording `JsError` per tab without crashing the browser.
   - Re-use `JsRuntime` event loop ticks (`step_timers()` / `drain_ready_timers()`) from `Browser::tick()` / platform loop.
4. **Dirty Tracking & Incremental Relayout/Repaint:**
   - Track granular dirty state (`DirtyFlags`: `CLEAN`, `PAINT` / `STYLE_AND_LAYOUT` / `TITLE`, or dirty subtree / root fallback) when JS mutates DOM (`textContent`, attributes, `classList`, `style`, child insertion/removal) or form state changes.
   - Only re-run `style_tree_with_context` + `layout_tree` when structure/style/layout actually dirtied; paint-only changes rebuild display list / repaint without redundant layout; idle ticks with no expired timers and no DOM mutations do **zero** relayout/repaint work.
5. **Memory & Safety Invariants:**
   - Iterative traversals only (no unbounded recursion on deep DOM trees).
   - Bounded console log buffer and timer/listener table caps so scripts cannot leak unbounded memory.
   - Preserve `ComputedStyle: Copy` and `ElementData.attributes: Vec<(String, String)>`.

## Verification

- Unit tests covering `document.getElementById`, `querySelector`/`querySelectorAll`, `createElement`/`appendChild`/`removeChild`, `textContent` (including Cyrillic UTF-8), `classList`, `style.*`, `setAttribute`/`value`/`checked`.
- Unit tests covering `addEventListener`, bubbling + `stopPropagation()`, `click`/`input`/`submit` with `preventDefault()`, timer callbacks mutating DOM, and dirty flag avoiding unnecessary relayout when no mutations occurred.
- Built-in interactive demo or script test in `droplet-shell` + full Global Verification Gate (`cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, CLI checks).

