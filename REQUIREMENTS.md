# Requirements

Rust port of the scrutiny-embedded C++ instrumentation library.

- C++ reference: https://github.com/scrutinydebuggerscrutiny-embedded/lib/
- Documentation: https://scrutinydebugger.com/

---

## Platform & Environment

- The library must compile and run on bare-metal embedded targets with no OS and no heap.
- The library must not depend on `std`. All functionality must be available in `no_std` context.
- The library must not perform any dynamic memory allocation.
- The library must be usable from a single-core system where `MainHandler` and `LoopHandler` instances run in different time domains (e.g. main loop vs. ISR).
- The library must be portable, adapting to different endianness, address size, register size, byte size.

---

## Safety & Correctness

- The library must not contain `unsafe` code except where strictly required by the IPC atomic primitive and raw pointer operations for memory read/write services.
- Every `unsafe` block must have a comment explaining why it is sound.
- The library must not panic in production code paths. All error conditions must be surfaced through return values.
- Callers must not be able to silently ignore error return values (`#[must_use]` on all fallible functions).
- Slices passed into the library are borrowed for the duration of the call only. The library does not store references to caller data beyond initialization.

---

## Initialization

- The library must be initializable without dynamic allocation. All internal state must fit in the `MainHandler` struct itself.
- `MainHandler::init()` must validate the configuration and return an error if it is incomplete or inconsistent. After a failed init, the handler must be inert (safe to call but does nothing).
- Configuration buffers (RX, TX, datalogging) are borrowed by `MainHandler` for its entire lifetime. The borrow checker enforces that the buffers outlive the handler at compile time.

---

## Communication

- The library exposes two byte streams: an input (`receive_data`) and an output (`pop_data` / `data_to_send`). It is the caller's responsibility to connect these to a physical transport (UART, USB CDC, CAN, etc.).
- The protocol layer must silently discard corrupted or incomplete packets without returning an error to the caller.
- If no byte is received for more than a configurable timeout (default 50ms), the communication state machine must reset to idle.
- If no heartbeat is received from the server within a configurable timeout (default 5 seconds) of connection, the session must be closed automatically.

---

## Timing

- All time values exchanged with the caller are in units of 100 nanoseconds (`u32`).
- `MainHandler::process()` must be called periodically by the application. The caller provides the elapsed time since the last call.
- `LoopHandler::process()` can be called from a different time domain than `MainHandler::process()` without the caller needing any synchronization primitive (mutex, critical section, interrupt disable).

---

## Loop Handlers

- The library must support multiple `LoopHandler` instances running at different frequencies.
- A `FixedFrequencyLoopHandler` has a constant timestep defined at construction. Its `process()` takes no timestep argument.
- A `VariableFrequencyLoopHandler` accepts a measured timestep at each `process()` call.
- Communication between `MainHandler` and each `LoopHandler` must be time-domain safe without locks, using a lock-free single-slot message mechanism.

---

## Runtime Published Values (RPVs)

- The library must support RPVs: named values identified by a 16-bit ID, with a type, readable and writable by the server without debug symbols.
- RPV read and write operations must be dispatched to caller-provided function pointer callbacks.
- The RPV array and callbacks are borrowed by `MainHandler` for its entire lifetime. The borrow checker enforces that they outlive the handler at compile time.

---

## Memory Access Services

- The library must support reading arbitrary memory regions on behalf of the server.
- The library must support writing arbitrary memory regions on behalf of the server, unless `memory_write_enable` is `false`.
- When the `protected_regions` feature is enabled, the library must refuse reads and writes that overlap a forbidden region, and refuse writes that overlap a read-only region.

---

## Datalogging (feature: `datalogging`)

- The library must support high-frequency data capture driven by a `LoopHandler`.
- The datalogger must operate on a caller-provided buffer. Buffer size determines the maximum acquisition duration.
- The datalogger must support trigger conditions that initiate the end of round robin acquisition.
- Datalogger state must be readable from the main handler in a thread-safe manner.
- Only one `LoopHandler` may own the datalogger at a time.

---

## Optional Features

- 64-bit variable types (`u64`, `i64`, `f64`) must be gated behind the `support_64bits` feature. When disabled, the `AnyType` storage type must be no wider than 4 bytes.
- Protected region enforcement must be gated behind the `protected_regions` feature.
- Datalogging must be gated behind the `datalogging` feature. When disabled, no datalogging code or data must be compiled in.

---

## C++ Implementation Divergence

The items below differ from the C++ reference on purpose. Do not "fix" them to match C++ unless explicitly asked. Wire compatibility with the Scrutiny server is preserved in all cases.

### Architecture adaptations

- The timebase is passed by argument on each call that needs it (e.g. `CommHandler::process(&Timebase)`) instead of being held by reference from init.
- The communication buffers are owned (borrowed for the handler lifetime) by `MainHandler`, not by `CommHandler`. `CommHandler` receives the RX/TX slices as call arguments.
- `MainHandler` and `CommHandler` interact through request metadata `(cmd, subfn, len)` and `active_response_mut()` instead of shared `Request`/`Response` pointers, to satisfy lifetime constraints.
- `Config` is consumed by `MainHandler::new()`, which returns `None` when buffers are missing. C++ copies the config and returns a disabled object with `Status::ERROR`.
- The datalogger lives in a `DataLoggerCell` owned by `MainHandler`; its address is transferred to the owning loop through the `TakeOwnership` IPC message, together with the RPV array and read callback.

### Features not ported

- No C wrapper (`cwrapper/`) and no CMake post-build tooling (`scrutiny_postbuild`, SFD generation, firmware tagging integration).
- `MainHandler` and `LoopHandler` does not expose the exact same public API because some internal components have restricted visibility.
- IPC uses a single `AtomicFlag` implementation (`StdAtomicFlag`, `AtomicBool`). The C++ per-platform IPC variants (AVR, TI C28, TriCore, x86) are not ported. The `LoopIpcFlag` alias in `ipc.rs` is the single place to change it.
- No `boolean16`, `boolean32`, `boolean64` variable types: a Rust `bool` is always 1 byte. Datalogging configurations using them are rejected with `InvalidRequest`.
- No `CHAR_BIT != 8` support (`GetParams` always reports `char_bit = 8`).
- No `SCRUTINY_DATALOGGING_BUFFER_32BITS` option: buffer sizes are `usize` and saturated to `u32` on the wire.

### Behavior differences

- **Datalogger ownership during reads**: `GetAcquisitionMetadata` and `ReadAcquisition` first request the owning loop to release the datalogger (returning `ProcessAgain` until released) while keeping the acquisition data. Consequences:
  - The owning loop must run within `REQUEST_MAX_PROCESS_TIME_US` (100 ms) or the request fails with `FailureToProceed`.
  - After a read, `ArmTrigger`/`DisarmTrigger` fail until the next `ConfigureDatalog`. The server always resets and reconfigures after a read, so this is compatible.
  - `ReadAcquisition` does not require an owner (C++ returns `FailureToProceed` without one).
- `DataAcquired` IPC messages update the main handler's cached datalogger state (C++ ignores them).
- Boolean values are read through the `uint8` union field, never through `bool`, because the byte may hold any value. In trigger comparisons any nonzero byte compares as 1; in acquisitions the raw byte is logged as-is.
- The trigger callback receives `Option<&'static str>` (the owning loop name) and the RPV read callback receives the loop name as caller, instead of a `LoopHandler*`.
- The session counter is per `CommHandler` instance (C++ uses a static shared by all instances).

---

## Testing

- Each module must have unit tests covering its behavior, compiled only under the `std` feature.
- The protocol codec must have tests verifying encoding and decoding of each service request and response.
- Integration tests must verify a full request/response exchange through `MainHandler` without real hardware.
- The public API of each feature should be tests in a test suite belonging in the "tests" folders. Only internal unit tests should eb aport of a module directly.
