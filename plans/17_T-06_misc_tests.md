# Plan 17: T-06, port the remaining miscellaneous tests

## Gap

Three C++ tests have no Rust equivalent:
- `TestIPC.CheckWithThread` (`../scrutiny-embedded/test/test_ipc.cpp`)
- `TestVariableFetching.RandomFetch` (`../scrutiny-embedded/test/test_variable_fetching.cpp`)
- `TestDataLoggingTypes.AnyTypeCompareCast` (`../scrutiny-embedded/test/datalogging/test_datalogging_types.cpp`)

## Context

- IPC: `IpcMessage<T, F: AtomicFlag>` in [src/ipc.rs](../src/ipc.rs) with `new_std`, `send`, `pop`, `has_content`; it is `Sync` for `T: Copy + Send`. Integration tests can use `std::thread`.
- Variable fetching: `fetch_variable`, `fetch_variable_bitfield`, `fetch_operand` in [src/datalogging/datalogger.rs](../src/datalogging/datalogger.rs) are `pub(crate)`, so tests go in that file's `#[cfg(test)] mod tests` (existing `fetch_*` tests are there). They take a `&dyn DataloggerHost`; existing tests already provide a test host.
- AnyType compare: C++ checks that `AnyTypeCompare` fields alias `AnyType` fields correctly. In Rust the equivalent is `convert_to_compare_type` plus `tools::read_biggest_*` / `set_biggest_*`. Existing tests: `tests/datalogging_types.rs`.

## What to do

1. `test_ipc_check_with_thread` (new in `tests/` or in `src/ipc.rs` tests under `std`): a producer thread sends N sequential values, each time waiting until `has_content()` is false; the consumer pops and checks it receives every value in order, with no loss or duplicate. Use a payload larger than one word (e.g. a struct of several `u32`) so a torn read would be detected. Mirror the C++ test's counts.
2. `random_fetch` in `datalogger.rs` tests: mirror `RandomFetch`. Read variables of each supported type from a buffer at various offsets through `fetch_variable`, compare against the expected value. Use a fixed-seed generator (no new dependency; a small xorshift in the test is fine).
3. `any_type_compare_cast` in `tests/datalogging_types.rs`: mirror `AnyTypeCompareCast`. After `set_biggest_uint`/`set_biggest_sint`/float writes, the matching `read_biggest_*` returns the same value, and `convert_to_compare_type` keeps values intact for the boundary values the C++ test uses.

## Acceptance

- New tests pass (64-bit cases under `support_64bits`). All checks listed in [00_README.md](00_README.md) pass.
