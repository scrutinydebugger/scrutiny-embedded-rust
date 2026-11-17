# C++ improvements found during the Rust port gap analysis

Changes to make later in the C++ repo (`../scrutiny-embedded`). Found while comparing it with the Rust port (see [../GAP_ANALYSIS.md](../GAP_ANALYSIS.md)). Nothing here has been changed in the C++ repo. File and line references are against the C++ repo at analysis time (2026-11-17).

## CPP-1 (D-02): DataLogControl when no loop supports datalogging

- **Where:** `lib/src/scrutiny_main_handler.cpp`, `process_datalog_control()` (around line 1404).
- **Today:** the guard is `if (!m_config.is_datalogging_configured()) return UnsupportedFeature;`, which only checks the buffer. When a datalogging buffer is set but no loop allows datalogging, `GetSupportedFeatures` reports datalogging as unsupported (it also checks `has_at_least_one_loop_with_datalogging()`), yet `GetSetup`, `GetStatus`, etc. still answer `OK`.
- **Suggested change:** use the same condition as `GetSupportedFeatures`: `is_datalogging_configured() && has_at_least_one_loop_with_datalogging()`. Every DataLogControl subfunction then returns `UnsupportedFeature` in that case. This is what the Rust port does.
- **Test to add:** buffer set, all loops with `allow_datalogging(false)` → every DataLogControl subfunction returns `UnsupportedFeature`.

## CPP-2 (D-07): GetLoopDefinition ignores the decode result

- **Where:** `lib/src/scrutiny_main_handler.cpp`, `process_get_info()`, case `GetLoopDefinition` (around line 783).
- **Today:** `m_codec.decode_request_get_loop_definition(request, &stack.get_loop_def.request_data);` — the returned code is ignored. With a request of the wrong length, `request_data.loop_id` is never written and the code reads an uninitialised stack value.
- **Suggested change:** `code = m_codec.decode_request_get_loop_definition(...); if (code != OK) break;` (same pattern as the other subfunctions). Result: `InvalidRequest` on a bad length.
- **Test to add:** GetLoopDefinition with payload length 0 and 2 → `InvalidRequest`.

## CPP-3 (D-08): Configure validation order for unknown condition / variable type

- **Where:** `lib/src/protocol/scrutiny_codec_v1_0.cpp`, `decode_datalogging_configure_request()` (around line 983); `lib/src/datalogging/scrutiny_datalogger.cpp`, `DataLogger::configure()`.
- **Today:** the decoder casts the trigger condition byte and the `Var`/`VarBit` datatype byte straight to the enum without checking them. Invalid values are only caught later in `DataLogger::configure()`, after the `loop_id` and protected-region checks. So a request that is wrong in several ways gets `FailureToProceed` or `Forbidden` instead of `InvalidRequest`, and an out-of-range value is stored in an enum.
- **Suggested change:** validate the condition (`< 9`) and the variable type (known `VariableType` value) in the decoder and return `InvalidRequest` immediately, as the Rust decoder does.
- **Test to add:** Configure with condition `0xFF` and an invalid `loop_id` → `InvalidRequest`. Configure with a `Var` operand of type `0xEE` → `InvalidRequest`.

## CPP-4 (D-09): VarBit operand with `bitsize == 0`

- **Where:** `lib/src/datalogging/scrutiny_datalogger.cpp`, `DataLogger::configure()`, `OperandType::VarBit` branch (around line 173).
- **Today:** `bitsize == 0` passes configuration and only fails at runtime in `MainHandler::fetch_variable_bitfield()`, so the trigger never fires and the acquisition silently never completes.
- **Suggested change:** add `if (bitsize == 0) m_config_valid = false;` so the Configure request is rejected with `InvalidRequest`. Note from the user: this may already be implemented in a branch of the C++ repo; check before doing it.
- **Test to add:** Configure with a `VarBit` operand of bitsize 0 → `InvalidRequest`.

## CPP-5 (D-10): ReadRPV calls the callback before checking for overflow

- **Where:** `lib/src/scrutiny_main_handler.cpp`, `process_memory_control()`, case `ReadRPV` (around line 1200); `ReadRPVResponseEncoder::write()` in `lib/src/protocol/scrutiny_codec_v1_0.cpp`.
- **Today:** for each RPV the read callback is called, then the encoder detects that the response doesn't fit and the request ends with `Overflow`. The user callback runs for a value that is never sent. If the callback has side effects (e.g. read-to-clear registers), they happen for nothing.
- **Suggested change:** check that `2 + type_size` fits in the remaining TX space before calling the callback, as the Rust port does.
- **Test to add:** ReadRPV whose response doesn't fit; check the callback is not called for the entry that overflows.
