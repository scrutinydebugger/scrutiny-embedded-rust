# Plan 10: F-08, clear loop IPC state when the MainHandler initialises

## Gap

C++ `MainHandler::init()` calls `LoopHandler::init()` on each loop, which clears both IPC channels and resets `m_datalogger_data_acquired`. Rust has no equivalent: a stale message left in a loop's IPC slot (e.g. after re-creating the `MainHandler`) is processed by the new handler. Decision: do the clear.

## Context

- Depends on plans 07 (F-06) and 09 (F-07). After plan 09, `MainHandler` holds `&'static [&'static LoopHandler]` (references to the core type); `init()` goes on the core `LoopHandler`.
- C++ reference: `LoopHandler::init()` in `../scrutiny-embedded/lib/src/scrutiny_loop_handler.cpp` (clears `m_main2loop_msg`, `m_loop2main_msg`; sets `m_datalogger_data_acquired = false`; stores the datalogger pointer). Called from `MainHandler::init()` before the datalogger init.
- Rust:
  - `IpcMessage` ([src/ipc.rs](../src/ipc.rs)) has `send`, `pop`, `has_content`, but no `clear`.
  - Loops are `&'static [LoopHandler]` from `Config::get_loops()`; `MainHandler::new` reads them.
  - Loop-side runtime state is `dl_runtime: UnsafeCell<LoopDlRuntime>` (`owns`, `data_acquired`, `cell`, `rpvs`, `rpv_read_cb`), written only by the loop task.
- Same precondition as C++: `MainHandler` initialisation happens before the loop tasks run (or while they are stopped). Writing loop-side state from `MainHandler::new` is only sound under that precondition.

## What to do

1. Add `pub(crate) fn clear(&self)` to `IpcMessage`: stores `false` in the flag. Doc comment.
2. Add `pub(crate) fn init(&self)` to `LoopHandler`:
   - clear `main2loop` and `loop2main` (when they exist under the current feature set);
   - under `datalogging`, reset `dl_runtime` to `LoopDlRuntime::DEFAULT` through the `UnsafeCell`, with a `SAFETY` comment stating the precondition above. This falls under the project's allowed unsafe category "datalogger ownership transfer between time domains".
3. In `MainHandler::new`, call `init()` on every loop before creating the datalogger cell.
4. Document the precondition in the doc comment of `MainHandler::new` ("Must be called before the loop handlers start running").

## Tests

- Unit test in `ipc.rs`: `send` then `clear` → `has_content() == false`, `pop() == None`.
- Integration test (datalogging feature): put a stale `Loop2MainMsg` (e.g. `OwnershipTaken`) in a static loop's `loop2main` before calling `MainHandler::new` (needs a `pub(crate)` path, so write it as a unit test in `src/main_handler.rs`), then check that after `new()` + `process(0)` the handler does not report `datalogging_ownership_taken()`.

## Acceptance

- All checks listed in [00_README.md](00_README.md) pass.
