# Plan 09: F-07, split loop handlers into fixed and variable frequency types

## Gap

C++ has two loop classes: `FixedFrequencyLoopHandler` has `process()` (steps by its configured timestep) and `process(timestep_100ns)`; `VariableFrequencyLoopHandler` has only `process(timestep_100ns)`. Rust has a single `LoopHandler` with only `process(&self, timestep)`.

Decisions from the user:
- `process()` with no argument is the default and steps by the configured timestep.
- `process_with_time(timestep_100ns)` receives an explicit timestep.
- Variable-frequency loops must **not** have `process()`; this must be enforced at compile time by splitting into two types (mirrors C++).

## Context

- Depends on plan 07 (F-06): `process` exists with and without the `datalogging` feature. Run this plan **before** plan 08 (D-01), so the datalogging builder lands on the new types.
- C++ reference: `../scrutiny-embedded/lib/inc/scrutiny_loop_handler.hpp` and `src/scrutiny_loop_handler.cpp`. The base `LoopHandler` holds the timebase, IPC, name and datalogging state, and `process_common(timestep)`. The two subclasses add the `process` overloads and `loop_type()` / `get_timestep_100ns()`. `Config::set_loops(LoopHandler** loops, count)` takes pointers to the base class, so both kinds share one array.
- Rust today ([src/loop_handler.rs](../src/loop_handler.rs)):
  - one `LoopHandler` struct with `const fn fixed_freq(name, timestep)` / `const fn variable_freq(name)`, builder `with_datalogging()` (becomes `without_datalogging()` in plan 08);
  - `Config::set_loops(&'static [LoopHandler])` ([src/config.rs:224](../src/config.rs#L224)); `MainHandler` stores `Option<&'static [LoopHandler]>` and indexes it for IPC, `GetLoopCount`, `GetLoopDefinition` (uses `loop_type()`, `get_timestep_100ns()`, `get_name()`, `datalogging_allowed()`), and the datalog Configure (loop id).
  - Users declare loops in `static` arrays, and tests call `loop.process(0)` from the same array.

## Target design

1. **Shared core:** keep `LoopHandler` as the common core (name, loop type, timestep, timebase, IPC, datalogging runtime). It keeps the read-only getters (`get_name`, `loop_type`, `get_timestep_100ns`, `datalogging_allowed`). Its processing function becomes `pub(crate) fn process_common(&self, timestep: Timediff)` (the body of today's `process`). Its constructors become `pub(crate)`.
2. **Two public wrapper types** in `loop_handler.rs`, each a thin newtype around the core:
   - `pub struct FixedFreqLoopHandler(LoopHandler);`
     - `pub const fn new(name: &'static str, timestep_100ns: u32) -> Self`
     - `pub fn process(&self)` → `self.0.process_common(self.0.get_timestep_100ns())`
     - `pub fn process_with_time(&self, timestep_100ns: Timediff)` → `self.0.process_common(timestep_100ns)`
   - `pub struct VariableFreqLoopHandler(LoopHandler);`
     - `pub const fn new(name: &'static str) -> Self`
     - `pub fn process_with_time(&self, timestep_100ns: Timediff)`
     - no `process()`.
   - Both: `pub const fn handler(&self) -> &LoopHandler` to get the core reference for the config, and forward the getters if useful to users.
3. **Config takes references to the core,** like C++ `LoopHandler**`: `Config::set_loops(&mut self, loops: &'static [&'static LoopHandler])`. `MainHandler` stores `Option<&'static [&'static LoopHandler]>`; update every use (iteration, indexing, `get_loop_count`, `get_loops`).
4. **User code target** (put this in the `set_loops` doc comment and in `README.md`):
   ```rust
   static FAST: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Fast", 1000);
   static SLOW: VariableFreqLoopHandler = VariableFreqLoopHandler::new("Slow");
   static LOOPS: [&LoopHandler; 2] = [FAST.handler(), SLOW.handler()];
   // config.set_loops(&LOOPS);
   // In the fast task:  FAST.process();
   // In the slow task:  SLOW.process_with_time(dt);
   ```
   **Verify this compiles** (a `static` initializer calling a `const fn` that returns a reference into another `static`). If the toolchain rejects it, use the closest alternative that keeps the compile-time restriction, in this order:
   1. `static LOOPS: [&LoopHandler; 2] = [&FAST.0, &SLOW.0];` with the inner field made `pub` but documented as "use only to register the loop";
   2. a small `macro_rules!` helper that expands to the field access.

   Report which one you used.
5. **Datalogging builder:** if plan 08 is already done, move `without_datalogging()` onto both wrapper types (`pub const fn without_datalogging(self) -> Self`). Otherwise plan 08 will add it there.
6. Remove `LoopHandler::fixed_freq` / `variable_freq` from the public API (or make them `pub(crate)`). No compatibility aliases (pre-release crate).
7. Update all call sites in `src/` (unit tests) and `tests/`: loop declarations, `set_loops`, and every `loop.process(x)` call → `process_with_time(x)` (or `process()` where the test meant the fixed step).
8. Every new public item gets a one-line doc comment.

## Tests

In `src/loop_handler.rs` unit tests (must pass with and without the `datalogging` feature):
- `FixedFreqLoopHandler::new("L", 100)`: three calls to `process()` advance the timebase to 300; `process_with_time(50)` then gives 350. Read the timebase through a `pub(crate)` accessor (add one if needed; no public API for it).
- `VariableFreqLoopHandler::new("V")`: `process_with_time(123)` advances the timebase by 123.
- A compile-fail check is not required, but add a doc comment on `VariableFreqLoopHandler` stating there is no `process()`.

Integration:
- `tests/get_info.rs` `GetLoopCount` / `GetLoopDefinition` tests updated to the new declaration style still pass (loop type, timestep, name, datalogging bit).
- A mixed array (one fixed, one variable) registered through `set_loops` works for `GetLoopDefinition` on both indexes.

## Acceptance

- Variable-frequency loops have no `process()` method.
- All checks listed in [00_README.md](00_README.md) pass.
