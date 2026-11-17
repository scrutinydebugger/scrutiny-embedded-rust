# Plan 07: F-06, LoopHandler usable without the `datalogging` feature

## Gap

`LoopHandler::process()` only exists with the `datalogging` feature ([src/loop_handler.rs:198](../src/loop_handler.rs#L198)), and so do the loop's `timebase` and its IPC channels. User code that calls `loop.process(dt)` does not compile when the feature is off. Decision: a loop handler must exist and be callable without datalogging; it just doesn't drive a datalogger.

## Context

- C++ reference: `../scrutiny-embedded/lib/inc/scrutiny_loop_handler.hpp` and `src/scrutiny_loop_handler.cpp`. Without `SCRUTINY_ENABLE_DATALOGGING`, the C++ `LoopHandler` still has its timebase, both IPC channels (with empty message-ID enums), `init()`, `process_common()` (steps the timebase, pops/ignores messages), `get_name()`, `loop_type()`, `get_timestep_100ns()`.
- Rust today ([src/loop_handler.rs](../src/loop_handler.rs)):
  - `timebase: UnsafeCell<Timebase>`, `dl_runtime`, `main2loop`, `loop2main` are all `#[cfg(feature = "datalogging")]`.
  - `unsafe impl Sync for LoopHandler` is also gated, because without the feature there is no `UnsafeCell` field.
  - `Main2LoopMsg` / `Loop2MainMsg` in [src/ipc.rs](../src/ipc.rs) are datalogging-only.
- `MainHandler::process_loops` ([src/main_handler.rs:394](../src/main_handler.rs#L394)) already compiles with and without the feature.
- Plans 08, 09, 10 and 11 build on this one. Plan 09 (run next) splits `LoopHandler` into `FixedFreqLoopHandler` / `VariableFreqLoopHandler` and renames `process(dt)` to `process_with_time(dt)`. Keep the name `process` in this plan; don't anticipate the split.

## What to do

1. Make `timebase: UnsafeCell<Timebase>` unconditional, and make `unsafe impl Sync for LoopHandler` unconditional (update its `SAFETY` comment: the timebase is written only by the loop task in `process()`).
2. Make `pub fn process(&self, timestep: Timediff)` unconditional:
   - always step the timebase;
   - put the datalogging part (IPC handling, datalogger processing, status messages) in a `#[cfg(feature = "datalogging")]` block or a private helper `process_datalogging(&self, ...)` gated by the feature.
3. Keep the IPC channels gated with `datalogging` for now, since they only carry datalogging messages today. Plan 10 (F-08) and plan 11 (F-09) will touch them; keep the gating consistent (`process()` must compile in both configurations).
4. Optionally add `pub fn get_timestamp(&self) -> Timestamp` only if needed by tests; otherwise don't add API.
5. Remove `#[cfg(not(feature = "datalogging"))] let _ = loop_handler;` workarounds if they become unneeded.

## Tests

- Add a test that compiles without the feature: in `src/loop_handler.rs` unit tests, a `#[cfg(not(feature = "datalogging"))]` test that creates `LoopHandler::fixed_freq("L", 100)` and calls `process(100)` several times. Also a feature-independent test that runs in both builds.
- Verify with `cargo test --no-default-features` (add `--features std` if the test harness requires it; check `Cargo.toml`) in addition to the standard checks.

## Acceptance

- `cargo build --no-default-features` compiles code that calls `LoopHandler::process`.
- All checks listed in [00_README.md](00_README.md) pass.
