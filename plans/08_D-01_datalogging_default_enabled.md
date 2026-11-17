# Plan 08: D-01, datalogging enabled by default on loops

## Gap

C++ loops support datalogging by default (`m_support_datalogging(true)`, disabled with `allow_datalogging(false)`). Rust loops have it off by default and opt in with `with_datalogging()` ([src/loop_handler.rs:119](../src/loop_handler.rs#L119), [:154](../src/loop_handler.rs#L154)). Without opt-in, the server sees no datalogging. Decision: reverse the logic, enabled by default, explicit disable required.

## Context

- Depends on plans 07 (F-06) and 09 (F-07). After plan 09, user-facing loops are `FixedFreqLoopHandler::new(...)` and `VariableFreqLoopHandler::new(...)`, wrapping the core `LoopHandler`.
- C++ API: `allow_datalogging(bool)` and `datalogging_allowed()` in `../scrutiny-embedded/lib/inc/scrutiny_loop_handler.hpp`.
- Constructors are `const fn` used in `static` items, so the disable call must also be a `const fn` builder, on **both** wrapper types.
- `with_datalogging()` is used all over `tests/comm_datalog_control.rs` and `tests/get_info.rs`.

## What to do

1. Set `datalogging_allowed: true` in the core constructors.
2. Remove `with_datalogging()` (wherever plan 09 left it). Add `pub const fn without_datalogging(self) -> Self` on `FixedFreqLoopHandler` and `VariableFreqLoopHandler`, setting the core flag to `false`, with a one-line doc comment. (The name was approved by the user; the C++ equivalent is `allow_datalogging(false)`.)
3. Update all call sites:
   - delete `.with_datalogging()` everywhere;
   - add `.without_datalogging()` where a test needs a loop without datalogging (e.g. `test_configure_no_datalogging_support`, `test_loop_definition_fixed_freq_datalogging_disabled`, `test_supported_features_*` that expect the bit cleared). Read each test's intent before editing.
4. Update `README.md` / docs if they show `with_datalogging()`.

## Tests

- Unit test in `loop_handler.rs`: a new loop has `datalogging_allowed() == true`; after `without_datalogging()` it is `false`.
- Existing tests keep their meaning after the call-site update (they must pass unchanged otherwise).
- Plan 20 (T-09) adds the server-level test for the disabled case.

## Acceptance

- All checks listed in [00_README.md](00_README.md) pass.
