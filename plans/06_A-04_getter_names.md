# Plan 06: A-04, align datalogging getter names with C++

## Gap

The public datalogging getters on `MainHandler` don't match the C++ names. Decision: make them match.

| C++ (`scrutiny_main_handler.hpp`) | Rust today ([src/main_handler.rs](../src/main_handler.rs#L341-L377)) | Rust target |
|---|---|---|
| `get_datalogger_state()` → `DataLogger::State::eState` | `datalogging_state()` → `Option<State>` | `get_datalogger_state()` → `State` |
| `datalogging_data_available()` | `datalogging_data_available()` | unchanged |
| `datalogging_error()` | `datalogging_in_error()` | `datalogging_error()` |
| `datalogging_ownership_taken()` | `datalogging_ownership_taken()` | unchanged |

## Context

- C++ always has a datalogger object, so `get_datalogger_state()` returns a plain state; when nothing is configured it is `Idle`.
- In Rust, `DataloggingState::DEFAULT.cached_state` is `State::Idle` and stays `Idle` when no datalogger cell exists, so returning `self.dl.cached_state` directly gives the C++ result without the `Option`.
- At analysis time, no test used `datalogging_state()` or `datalogging_in_error()`; grep again before renaming.

## What to do

1. Rename `datalogging_state()` to `get_datalogger_state()` and return `State` (`self.dl.cached_state`). Update the doc comment: returns `Idle` when datalogging is not configured.
2. Rename `datalogging_in_error()` to `datalogging_error()`. Keep the body.
3. Update every call site (`src/`, `tests/`, `README.md`, examples if any).
4. Do not keep deprecated aliases (the crate is pre-release).

## Tests

- Add a small test: handler without datalogging buffer → `get_datalogger_state() == State::Idle` and `!datalogging_error()`.
- Add a check of `get_datalogger_state()` in an existing datalog test after Configure (expect `Configured` once the loop has run).

## Acceptance

- All checks listed in [00_README.md](00_README.md) pass.
