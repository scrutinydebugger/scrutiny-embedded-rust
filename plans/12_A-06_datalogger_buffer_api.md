# Plan 12: A-06, safe datalogger buffer API (if no re-architecture is needed)

## Gap

`Config::set_datalogger_buffer` is `unsafe` and takes a raw pointer plus a length ([src/config.rs:245](../src/config.rs#L245)). Decision from the user: the datalogger is shared across time domains without the compiler's knowledge, so some unsafe is expected. **If the API can be made safe without re-architecting the ownership model, do it; otherwise leave it as is** and explain why in [ACCEPTED_DIFFERENCES.md](../ACCEPTED_DIFFERENCES.md).

## Context

- `REQUIREMENTS.md` says: "Configuration buffers (RX, TX, datalogging) are borrowed by `MainHandler` for its entire lifetime. The borrow checker enforces that the buffers outlive the handler at compile time." The current raw-pointer API does not meet this for the datalogging buffer.
- The buffer is stored in `DataLoggerCell` (inside `MainHandler`) and then accessed from loop tasks, which are `&'static`. So the buffer must live at least as long as any loop can touch it. `'static` is the honest bound.
- Proposed safe API (analysis suggests this needs no ownership redesign; confirm while implementing):
  `pub fn set_datalogger_buffer(&mut self, buf: &'static mut [u8], trigger_cb: Option<DataloggingTriggerCallback>)`
  Internally, convert to `(*mut u8, usize)` exactly as today. Taking `&'static mut` guarantees exclusivity and lifetime at the call site; the existing internal `unsafe` (cell access guarded by the ownership token) is unchanged.
- Test code currently uses `static mut DL_BUF: [u8; N]` and passes a raw pointer. With the new API, tests need a `&'static mut [u8]`. Options: `unsafe { &mut *core::ptr::addr_of_mut!(DL_BUF) }` (one `unsafe` in test code, allowed) or a helper that leaks a `Box` in tests (tests may use `std`).
- **Accepted constraint (do not fix):** `DataLoggerCell` lives inside `MainHandler` by value, and loops receive its address (`cell_addr`) through `Main2LoopMsg::TakeOwnership`. Moving the `MainHandler` after a loop took ownership would leave a dangling pointer. The user accepted this: `MainHandler` is meant to be a global instance that lives for the whole program and is never moved. It is recorded in `ACCEPTED_DIFFERENCES.md`. As part of this plan, add a short paragraph to the doc comment of `MainHandler` (struct) and `MainHandler::new` stating that the handler must not be moved after the first `process()` call and is meant to live for the whole program.

## What to do

1. Try the safe signature above. Remove `unsafe` from `set_datalogger_buffer` and its `# Safety` section; keep a doc comment saying the buffer is shared with loop time domains, hence `'static`.
2. Keep `Config` fields as they are internally if that is simplest, or store `Option<&'static mut [u8]>` and convert in `take_datalogger_buf`.
3. Update all call sites in `tests/` and `src/` unit tests.
4. If it turns out a safe signature requires changing how the datalogger is owned or shared, stop, revert, and instead add an entry to `ACCEPTED_DIFFERENCES.md` (A-06) explaining the blocker.

## Tests

- Existing datalog tests must pass with the new signature.
- Add a compile-level check in a unit test that `set_datalogger_buffer` can be called without `unsafe` given a `&'static mut [u8]`.

## Acceptance

- Either the safe API is in place and all checks in [00_README.md](00_README.md) pass, or the reason it can't be done is recorded in `ACCEPTED_DIFFERENCES.md`.
- The "do not move the `MainHandler`" constraint is documented on `MainHandler` and `MainHandler::new`.
