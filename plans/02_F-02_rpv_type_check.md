# Plan 02: F-02 + T-10, validate RPV types at init

## Gap

C++ `MainHandler::check_config()` disables the handler if any Runtime Published Value (RPV) has an unsupported type. The Rust `MainHandler::new` only checks the RX/TX buffer sizes ([src/main_handler.rs:157-217](../src/main_handler.rs#L157-L217)). A side effect: Rust ReadRPV returns `FailureToProceed` and WriteRPV returns `InvalidRequest` when an RPV has a 0-size type, a path C++ never reaches.

## Context

- C++ reference: `MainHandler::check_config()` in `../scrutiny-embedded/lib/src/scrutiny_main_handler.cpp` (around line 103), and `tools::is_supported_type()` in `../scrutiny-embedded/lib/src/scrutiny_tools.cpp` (around line 89).
  - Supported: native `boolean`, booleans whose size equals the platform `bool` size, `uint`/`sint` of 1, 2, 4 bytes (8 with 64-bit support), `float` of 4 bytes (8 with 64-bit support).
  - Unsupported: `float8`, `float16`, any `cfloat`, sizes above the 64-bit limit, `unknown`.
  - On failure, C++ calls `m_comm_handler.disable()`, sets `m_enabled = false` and `init` returns `ERROR`. The object stays alive but inert.
- Rust already has the same predicate: `is_supported_type()` in [src/datalogging/datalogger.rs:761](../src/datalogging/datalogger.rs#L761), but it is only compiled with the `datalogging` feature.
- Current Rust behaviour for bad buffer sizes: `new()` returns `Some(handler)` with `is_enabled() == false` (comes from `comm.is_enabled()`). `new()` returns `None` only when a buffer is missing (accepted as A-03).
- `CommHandler::disable(&mut self, tb: &Timebase)` exists.

## What to do

1. Move `is_supported_type()` (and its 64-bit helper) from `datalogging/datalogger.rs` to `src/tools.rs` as `pub fn is_supported_type(vt: VariableType) -> bool`, compiled without feature gate. Update the datalogger to use `crate::tools::is_supported_type`. Keep its existing unit tests passing; move them if they test the function directly.
2. In `MainHandler::new`, after creating the comm handler, check every RPV in `config.get_rpvs_array()`. If any type is unsupported:
   - set `enabled = false`;
   - call `comm.disable(&timebase)` (create the timebase before, or use a temporary `Timebase::new()`);
   - still return `Some(handler)`, so the handler is inert like with bad buffer sizes (matches C++ and `REQUIREMENTS.md`: "After a failed init, the handler must be inert").
3. Keep the ReadRPV/WriteRPV 0-size checks in `codec_v1_0.rs` as defensive code; they are now unreachable through valid configs. Do not turn them into panics.

## Tests (T-10)

Add to `src/main_handler.rs` unit tests or a new `tests/init.rs`:

- `init_rejects_unsupported_rpv_type`: one RPV per unsupported type (`Float8`, `Float16`, `Cfloat8`, `Cfloat16`, `Cfloat32`, and `Cfloat64` with `support_64bits`). For each, `MainHandler::new(cfg)` returns `Some(h)` with `!h.is_enabled()`, and a Discover request receives no response (`data_to_send() == 0`).
- `init_accepts_supported_rpv_types`: one RPV of each supported type (`Boolean`, `Boolean8`, `Sint8/16/32`, `Uint8/16/32`, `Float32`, plus 64-bit types under `support_64bits`). Handler is enabled.
- Optional: one valid and one invalid RPV in the same array → disabled.

## Acceptance

- New tests pass under `--all-features` and `cargo build --no-default-features` still compiles (the moved function must not depend on `datalogging`).
- All checks listed in [00_README.md](00_README.md) pass.
