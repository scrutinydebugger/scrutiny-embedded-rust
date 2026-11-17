# Accepted differences with the C++ reference

This crate is a port of `scrutiny-embedded` (C++). The differences below are **intentional**. Do not "fix" them to match the C++ code. Item IDs refer to [GAP_ANALYSIS.md](GAP_ANALYSIS.md) (analysis of 2026-11-17).

If you find a new difference, decide whether it is a bug or a choice, and add it here if it is a choice.

## Architecture

| ID | Difference | Reason |
|---|---|---|
| – | Timebases are passed by function call (`tb: &Timebase`) instead of being stored by reference at init. | Rust lifetime rules. |
| – | Communication buffers are owned by `MainHandler`, not by the `CommHandler`. `CommHandler` methods take the buffers as arguments. | Rust lifetime rules. |
| – | The `MainHandler` / `CommHandler` interaction differs (e.g. `get_request()` returns metadata, the payload stays in the caller's buffer). | Rust lifetime rules. |
| – | `MainHandler` must not be moved after its first `process()` call: it owns the datalogger, and loops receive the datalogger's address when they take ownership. `MainHandler` is meant to be a global instance that lives for the whole run of the program. | Accepted by the user. Moving it would need a different ownership model for the datalogger. |
| D-03 | `GetAcquisitionMetadata` and `ReadAcquisition` first ask the owning loop to release the datalogger (answering `ProcessAgain` until it does), then read it. The data is preserved by that release. After a read, `ArmTrigger`/`DisarmTrigger` return `FailureToProceed` until the next Configure. If the loop doesn't release within `REQUEST_MAX_PROCESS_TIME_US`, the request fails with `FailureToProceed`. | Required for a sound ownership transfer between time domains. Compatible with the server, which always runs Configure → Arm → Status → Metadata → Read → Reset and never re-arms after a read (`scrutiny-main/scrutiny/server/device/submodules/datalogging_poller.py`). |

## Features not ported

| ID | Difference | Reason |
|---|---|---|
| F-03 | No `Boolean16`, `Boolean32`, `Boolean64` variable types. Requests that use them (`0x31`, `0x32`, `0x33`) are rejected with `InvalidRequest`. | A Rust `bool` is always 1 byte. |
| F-04 | No public access to the datalogger object (C++ `MainHandler::datalogger()`), so no user-side `force_trigger()`. | Decision: leave as is. |
| F-05 | `MainHandler` does not expose `get_rpv`, `rpv_exists`, `get_rpv_type`, `get_rpv_read_callback`, `fetch_variable`, `fetch_variable_bitfield`, `comm()`, `get_config()`. | Decision: leave as is. |
| F-09 | Only `StdAtomicFlag` is used for loop IPC (no AVR / TriCore / TI C28 / x86 variants). The flag type is kept replaceable through the `AtomicFlag` trait. | Enough for now; extension point kept. |
| F-10 | No compile-time check that `DATALOGGING_MAX_SIGNAL <= 254`. | Decision: do nothing. |
| F-11 | No `SCRUTINY_DATALOGGING_BUFFER_32BITS` option. | Buffer sizes are `usize`; protocol fields are `u32`. |
| F-12 | No C wrapper, no `CHAR_BIT == 16` support, no `make_address_range` helpers. | Out of scope for the Rust port. |

## Behaviour

| ID | Difference | Reason |
|---|---|---|
| D-02 | When a datalogging buffer is set but no loop allows datalogging, every DataLogControl subfunction returns `UnsupportedFeature` (C++ still answers `GetSetup` and others). | Consistent with `GetSupportedFeatures`. Reported as a C++ improvement (`plans/CPP_IMPROVEMENTS.md`, CPP-1). |
| D-04 | The `DataAcquired` loop message sets the cached state to `AcquisitionCompleted` immediately (C++ waits for the next status update). | Harmless; slightly faster status. |
| D-05 | `SOFTWARE_ID` is a `#[no_mangle] pub static`. Whether the firmware-ID tagging tool works on Rust binaries has not been verified (possible constant folding). | Decision: ignore for now. |
| D-07 | `GetLoopDefinition` with a wrong payload length returns `InvalidRequest` (C++ reads an uninitialised value). | Rust is correct; reported as CPP-2. |
| D-08 | Unknown trigger condition or variable type in Configure is rejected while decoding, so it returns `InvalidRequest` before the `loop_id` check. | Rust is stricter; reported as CPP-3. |
| D-09 | A `VarBit` operand with `bitsize == 0` is rejected at Configure. | Rust is stricter; reported as CPP-4. |
| D-10 | ReadRPV checks the remaining TX space before calling the read callback (C++ calls it first). | Avoids useless callback side effects; reported as CPP-5. |
| D-11 | Bitfield fetch with a boolean type and more than 8 bits fails (only `Boolean8` output). | On purpose; follows from F-03. |
| Q-02 | Display name (64 bytes) and loop name (32 bytes) are truncated on a byte boundary, which can split a UTF-8 character. | On purpose, same as C++. Keep names short or ASCII. |

## API

| ID | Difference | Reason |
|---|---|---|
| A-01 | `RpvReadCallback` and the datalogging trigger callback receive the loop name (`Option<&'static str>`, `None` from the main handler) instead of a `LoopHandler*`. Loop names are not checked for uniqueness. | On purpose. |
| A-02 | `RpvWriteCallback` has no `caller` argument. | On purpose. |
| A-03 | `MainHandler::new(config)` consumes the config and returns `Option<MainHandler>`: `None` when a buffer is missing, `Some` with `is_enabled() == false` for other invalid configurations. | Rust idiom. |
| A-05 | The session counter is per `MainHandler` instance (static in C++). | Decision: leave as is. |
| A-07 | `UserCommandCallback` is `fn(subfunction: u8, request: &[u8], response: &mut [u8], response_len: &mut u16)`; the maximum response length is `response.len()`. | Rust idiom. |
| – | Loops mirror the C++ class split with composition instead of inheritance: `FixedFreqLoopHandler` (`process()`, `process_with_time(dt)`) and `VariableFreqLoopHandler` (`process_with_time(dt)` only) wrap a shared core `LoopHandler`, and `Config::set_loops` takes `&'static [&'static LoopHandler]` (C++ `LoopHandler**`). The C++ `process(timestep)` overload is named `process_with_time`. | Rust has no inheritance or overloading. (Applies once plan 09 is done.) |
| – | Callbacks are function pointers, not closures. | `CLAUDE.md` rule. |
