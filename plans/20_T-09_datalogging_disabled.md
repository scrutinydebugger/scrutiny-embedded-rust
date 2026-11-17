# Plan 20: T-09, test loops with datalogging disabled

## Gap

No test covers the case "datalogging buffer configured, but no loop allows datalogging". In Rust, `MainHandler::new` then creates no datalogger, `GetSupportedFeatures` clears the datalogging bit, and every DataLogControl subfunction returns `UnsupportedFeature`. C++ still answers `GetSetup` in that case (accepted difference D-02, reported in `plans/CPP_IMPROVEMENTS.md`).

## Context

- Depends on plan 08 (D-01): loops are datalogging-enabled by default and use `.without_datalogging()` to disable.
- Code: [src/main_handler.rs:174-186](../src/main_handler.rs#L174-L186) (datalogger created only if a loop allows it), `dispatch()` for `DataLogControl`, `process_get_info` (`GetSupportedFeatures`, `GetLoopDefinition` attribute bit `0x80`).
- Helpers: `tests/comm_datalog_control.rs`, `tests/get_info.rs`.

## What to do

1. `test_datalogging_disabled_on_all_loops` in `tests/comm_datalog_control.rs`: two loops, both `.without_datalogging()`, datalogging buffer set.
   - `GetSupportedFeatures`: bit `0x40` clear.
   - Every DataLogControl subfunction 1–8 → `UnsupportedFeature`.
   - `get_datalogger_state()` (name from plan 06; use the current name if plan 06 is not done) returns `Idle`.
2. `test_datalogging_disabled_on_one_loop`: loop 0 enabled, loop 1 disabled.
   - `GetSupportedFeatures`: bit `0x40` set.
   - `GetLoopDefinition` for loop 1: attribute bit `0x80` clear; loop 0: set.
   - Configure targeting loop 1 → `Forbidden`.
3. Make sure the default (no call) gives datalogging enabled: `GetLoopDefinition` attribute bit set for a loop created with `FixedFreqLoopHandler::new(...)` only, and the same for `VariableFreqLoopHandler::new(...)`.

## Acceptance

- New tests pass. All checks listed in [00_README.md](00_README.md) pass.
