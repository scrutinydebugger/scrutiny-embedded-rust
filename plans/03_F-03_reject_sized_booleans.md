# Plan 03: F-03, make sure boolean16/32/64 are rejected

## Gap

The C++ `VariableType` has `boolean16` (`0x31`), `boolean32` (`0x32`) and `boolean64` (`0x33`). Rust does not, on purpose: a Rust `bool` is always 1 byte. Decision: do not add them, but make sure a server request that uses them is rejected cleanly and covered by tests.

## Context

- Type code = type-type (`boolean = 3 << 4 = 0x30`) | size (`16 → 1`, `32 → 2`, `64 → 3`). Values confirmed in `../scrutiny-embedded/lib/inc/scrutiny_c_compatible_types.h`.
- The only place where the server sends a variable type to the device is the datalog Configure request, for `Var` and `VarBit` trigger operands. `decode_datalogging_configure_request` in [src/protocol/codec_v1_0.rs](../src/protocol/codec_v1_0.rs#L1035) uses `VariableType::try_from`, which returns `Err` for these codes, and the decoder returns `InvalidRequest`. RPV types are defined by the user in Rust, so they can't hold these values. Memory reads/writes carry no type.
- Expected result: no code change needed in the decoder. This plan adds the tests and a code comment.

## What to do

1. In [src/types.rs](../src/types.rs), add a short comment near `Boolean8` / `Boolean` saying `boolean16/32/64` (0x31–0x33) are not supported because a Rust `bool` is 1 byte, and that `try_from` rejects them.
2. Add a unit test in `src/types.rs` (or `tests/datalogging_types.rs`): `VariableType::try_from(0x31 | 0x32 | 0x33)` returns `Err(())`.
3. Add integration tests in `tests/comm_datalog_control.rs`, using the existing `send_configure` / `DlConfig` helpers:
   - `test_configure_operand_var_sized_boolean_rejected`: a `Var` operand with datatype `0x31`, then `0x32`, then `0x33` → response code `InvalidRequest`; datalogger state afterwards is not `Configured` (check with GetStatus).
   - `test_configure_operand_varbit_sized_boolean_rejected`: same for a `VarBit` operand.

## Acceptance

- New tests pass. All checks listed in [00_README.md](00_README.md) pass.
