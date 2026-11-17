# Plan 21: T-11, end-to-end acquisition with the loop on another thread

## Gap

All datalogging tests run the `MainHandler` and the `LoopHandler` on the same thread, alternating calls. Nothing exercises the real use case: the loop running in another time domain while the main handler talks to the server. The ownership transfer and the IPC (`DataLoggerCell`, `IpcMessage`) are the unsafe parts of the crate, so they need this coverage.

## Context

- Depends on plans 08 (D-01) and 09 (F-07) for the loop types and constructors.
- Loop handlers are `Sync` and declared as `static`; `FixedFreqLoopHandler::process()` can be called from a spawned `std::thread` (integration tests link `std`).
- `MainHandler` is not `Send` (holds `&mut` buffers); keep it on the test's main thread.
- The datalogger cell is inside the `MainHandler`, and loops receive its address. **Do not move the `MainHandler` after the first `process()` call.** This is an accepted constraint (it is meant to be a global instance, see `ACCEPTED_DIFFERENCES.md`). Create it, then only use it through `&mut`.
- Logged variables and the trigger operand must be statics written by the loop thread (e.g. `AtomicU32` storing `f32` bits, or `static mut` with `addr_of_mut!` and a clear comment), so the datalogger reads changing values from memory.

## What to do

New file `tests/datalogging_threaded.rs`:

1. A static `FixedFreqLoopHandler` (timestep 100) registered through a static `[&LoopHandler; 1]` array, a static datalogging buffer, static logged variables.
2. Spawn a thread that, until a stop flag (`AtomicBool`) is set: updates the logged variable (e.g. a counter), calls `LOOP.process()`, and yields or sleeps briefly.
3. On the main thread, drive the server sequence with `MainHandler` (copy helpers from `tests/comm_datalog_control.rs`): connect → Configure (trigger on the counter crossing a threshold, log the counter and Time) → Arm → poll GetStatus until `AcquisitionCompleted` (bounded number of iterations, fail with a clear message on timeout) → GetAcquisitionMetadata → ReadAcquisition until finished → ResetDatalogger. Retry on `ProcessAgain` by calling `process()` again; never busy-wait without a bound.
4. Check:
   - CRC of the read data matches;
   - logged counter values are strictly increasing within the acquisition;
   - the number of points and the trigger position match the metadata.
5. Stop and join the thread at the end.
6. Repeat the whole sequence 2–3 times in the same test to cover re-configuration across threads.

## Acceptance

- The test passes reliably (run it 20 times in a loop locally: `for i in $(seq 20); do cargo test --all-features --test datalogging_threaded || break; done`).
- All checks listed in [00_README.md](00_README.md) pass.
