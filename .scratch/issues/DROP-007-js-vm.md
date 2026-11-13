# DROP-007: Minimalist ES Subset Parser & Bytecode VM

- **Phase:** 7
- **Status:** `in-progress`
- **Blocked By:** `DROP-002`, `DROP-003` (both `done`)
- **Implementation owner:** DeepSeek V4.1 Flash Free (OpenCode agent)

---

## Goal

A self-contained, zero-dependency JavaScript subset compiled to bytecode and run
by a compact VM. DOM bindings are explicitly **out of scope** (DROP-008).

## Required scope (new crate `crates/droplet-js`)

1. **Lexer** — identifiers (ASCII + Unicode letters), numbers (int/float/hex),
   strings with escapes (including `\u{...}`/`\uXXXX`), template-free, comments
   (`//`, `/* */`), all common punctuation/operators, UTF-8 safe.
2. **Parser** — `var`/`let`/`const`, function declarations and expressions,
   blocks, `if`/`else`, `while`, `for` (C-style), `break`/`continue`, `return`,
   expression statements; literals, arrays, objects, member/index access, calls,
   unary/binary/logical, assignment and compound assignment, `++`/`--`,
   ternary. Clear line/column errors.
3. **Compiler** — emit explicit bytecode instructions (not an AST walker) with
   constant pool, local slots and jump patching. Add a disassembler for tests.
4. **VM** — values: `undefined`, `null`, bool, number, string, array, object,
   function/closure, native function. Environments with lexical scope and
   closures; recursion; object/array mutation; truthiness/equality/coercion
   rules sufficient for the subset; `console.log`.
5. **Event loop** — `setTimeout`, `setInterval`, `clearTimeout`, `clearInterval`
   with a deterministic virtual clock API (`tick(now_ms)`) for tests; bounded
   pending-timer count.
6. **Shell integration** — collect inline `<script>` (and external `<script
   src>` when practical) from the loaded page, execute in order on load, capture
   `console.log`/errors into the status bar or a small log, and never panic on
   syntax/runtime errors. Keep DOM APIs out; leave a clean hook for DROP-008.
7. **Tests** — lexer/parser errors, arithmetic/strings (Cyrillic), closures,
   recursion, arrays/objects, loops/break/continue, timers, stack/step limits,
   and a headless script execution test.

## Constraints

- Zero dependencies and no proc macros.
- Bytecode VM (register or stack); no `eval`-style AST interpreter.
- Hard limits on call depth, instruction budget and timer count to avoid hangs.
- All string operations respect UTF-8 boundaries.
- Do not edit forms, history/find or platform backends.
