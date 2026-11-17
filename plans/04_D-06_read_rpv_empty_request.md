# Plan 04: D-06, match C++ behaviour for ReadRPV edge cases

## Gap

An empty `MemoryControl/ReadRPV` request (data length 0) returns `InvalidRequest` in C++ but `OK` with no data in Rust ([src/protocol/codec_v1_0.rs:753](../src/protocol/codec_v1_0.rs#L753)). Decision: replicate C++ and test it.

## Context

- C++ reference in `../scrutiny-embedded/lib/src/protocol/scrutiny_codec_v1_0.cpp`:
  - `ReadRPVRequestParser::init` marks the request invalid only if the length is odd.
  - `MainHandler::process_memory_control` loops `while (!parser->finished())`. With length 0, the first `next()` finds `2 > 0` remaining bytes and sets `m_invalid`, so the result is `InvalidRequest`.
  - `WriteRPVRequestParser` behaves the same; Rust already returns `InvalidRequest` for an empty WriteRPV.
- Closely related difference in the same functions: C++ returns `UnsupportedFeature` unless `Config::is_read_published_values_configured()` (or `is_write_...`) is true, which requires a callback **and** at least one RPV. Rust checks `rpvs.is_some()` and the callback, but an empty RPV slice (`Some(&[])`) passes. Fix it in the same change so both match C++.

## What to do

In [src/protocol/codec_v1_0.rs](../src/protocol/codec_v1_0.rs):

1. `encode_response_memory_control_read_rpv`:
   - Return `UnsupportedFeature` when `rpvs` is `None` **or empty**, or when `read_cb` is `None` (keep the order: feature check first, as in C++).
   - Return `InvalidRequest` when `data.is_empty()` (in addition to the odd-length check).
2. `encode_response_memory_control_write_rpv`: return `UnsupportedFeature` when `rpvs` is `None` **or empty**, or when `write_cb` is `None`. The empty-data check already exists.
3. Update the `# Errors` doc sections.

## Tests

In `tests/comm_memory_control.rs`:

- `test_read_rpv_empty_request`: RPVs and read callback configured, ReadRPV with empty payload → `InvalidRequest`.
- `test_read_rpv_empty_rpv_list`: `set_published_values(&[], Some(cb), None)` → ReadRPV returns `UnsupportedFeature`.
- `test_write_rpv_empty_rpv_list`: same for WriteRPV.
- `test_write_rpv_empty_request` if not already covered → `InvalidRequest`.

## Acceptance

- New tests pass; existing RPV tests still pass. All checks listed in [00_README.md](00_README.md) pass.
