# Plan 13: T-02, port missing MemoryControl RPV tests

## Gap

Five C++ tests from `../scrutiny-embedded/test/commands/test_memory_control_rpv.cpp` have no Rust equivalent.

## Context

- Depends on plan 04 (D-06), which changes RPV edge-case behaviour.
- Target file: `tests/comm_memory_control.rs` (existing RPV tests: `test_read_rpv_*`, `test_write_rpv_*`). Reuse its helpers and RPV callbacks; add statics as needed (callbacks are `fn` pointers, so test state goes in statics).
- Rust encoding facts: RPV value encoding is big-endian; `Boolean` RPVs are reported as `Boolean8` (1 byte); write response is `id(2) + type_size(1)` per entry; read response is `id(2) + value(type_size)` per entry.
- Rust checks for TX overflow **before** calling the read callback (accepted difference D-10, see `ACCEPTED_DIFFERENCES.md`); C++ calls the callback first. Tests must not assert on callback call counts in overflow cases.

## What to do

Read each C++ test, then port it to Rust with the same request bytes and expected results, adapting only what Rust intentionally does differently:

1. `test_read_multiple_rpv_each_type` (C++ `TestReadMultipleRPVEachType`): one RPV per supported type (64-bit types under `#[cfg(feature = "support_64bits")]`), read them all in one request, check every value and its encoding.
2. `test_read_rpv_response_overflow` (`TestReadRPVResponseOverflow`): TX buffer too small for the response → `Overflow`, empty payload.
3. `test_write_rpv_all_types` (`TestWriteAllTypes`): write one RPV of each type, check the write callback receives the correct decoded value and type, and the response lists `id + size` for each.
4. `test_write_rpv_response_overflow` (`TestWriteRPVResponseOverflow`): → `Overflow`.
5. `test_write_rpv_response_full_no_overflow` (`TestWriteRPVResponseFullNoOverflow`): response exactly fills the TX buffer → `OK`.

For overflow tests, choose a TX buffer size at or above `MINIMUM_TX_BUFFER_SIZE` (32), otherwise the handler is disabled.

## Acceptance

- Five new tests pass. All checks listed in [00_README.md](00_README.md) pass.
