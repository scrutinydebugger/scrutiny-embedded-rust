# Plan 11: F-09, keep the IPC flag extensible

## Gap

C++ selects a platform IPC implementation at build time (AVR, TriCore, TI C28, x86, std atomic). Rust has an `AtomicFlag` trait ([src/ipc.rs](../src/ipc.rs)), but `LoopHandler` hard-codes `IpcMessage<_, StdAtomicFlag>` ([src/loop_handler.rs:99-101](../src/loop_handler.rs#L99-L101)) and constructs it with `IpcMessage::new_std`. Decision: keep `StdAtomicFlag` for now, but make sure another flag can be plugged in later without touching every use site.

## Context

- Depends on plans 07 and 10 (same files).
- Places that name the flag type today: `LoopHandler` fields, `main2loop()` / `loop2main()` return types, the two constructors (`IpcMessage::new_std`), and `main_handler.rs` through those accessors.
- Making `LoopHandler` generic over the flag would spread a type parameter into `Config`, `MainHandler` and every user `static` array. That is a larger API change than requested; do not do it now.

## What to do

1. In `src/ipc.rs`, add a crate-level alias with a doc comment:
   `pub type LoopIpcFlag = StdAtomicFlag;` and a comment saying this is the single place to change (or later make `cfg`-selectable) to use another platform flag.
2. Add a generic `const` constructor if possible: `IpcMessage::<T, LoopIpcFlag>::new_loop(init)` or simply keep `new_std` but route all `LoopHandler` code through the alias, so the only `StdAtomicFlag` mention outside `ipc.rs` disappears.
3. Replace `StdAtomicFlag` with `LoopIpcFlag` in `loop_handler.rs` (fields and accessor return types).
4. Note in the `AtomicFlag` trait doc how a platform would provide its own flag (implement the trait, provide a `const fn new`, change `LoopIpcFlag`).

## Tests

- Existing `ipc.rs` tests (`custom_atomic_flag_impl`) must still pass; they show a custom flag works with `IpcMessage`.
- No new behaviour; build and existing tests are enough.

## Acceptance

- `grep StdAtomicFlag src/` only matches `ipc.rs`.
- All checks listed in [00_README.md](00_README.md) pass.
