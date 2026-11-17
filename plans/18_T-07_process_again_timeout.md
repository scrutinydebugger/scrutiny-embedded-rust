# Plan 18: T-07, test the ProcessAgain timeout

## Gap

No test covers what happens when a request keeps returning `ProcessAgain` for longer than `REQUEST_MAX_PROCESS_TIME_US`. In both C++ and Rust, the request must then be answered with `FailureToProceed`. This matters more in Rust, because datalogging reads now wait for the loop to release the datalogger (D-03, see `ACCEPTED_DIFFERENCES.md`).

## Context

- Logic in `MainHandler::process` ([src/main_handler.rs:263-280](../src/main_handler.rs#L263-L280)): first `ProcessAgain` stores a timestamp; later ones check `has_expired(timestamp, REQUEST_MAX_PROCESS_TIME_US * 10)` and then send `FailureToProceed`.
- `process(timestep_100ns)` advances the main timebase.
- Ways to produce `ProcessAgain` in Rust (all in `process_datalog_control`): `ConfigureDatalog` or `ResetDatalogger` while a loop owns the datalogger, or `GetAcquisitionMetadata` / `ReadAcquisition` while a loop owns a completed acquisition. If the loop is never processed (`process()` / `process_with_time(dt)` after plan 09), it never releases, so `ProcessAgain` repeats.
- Helpers in `tests/comm_datalog_control.rs`: `make_handler_with_loops`, `send_configure`, `connect`, `make_request`, `pop_response`.

## What to do

Add to `tests/comm_datalog_control.rs`:

1. `test_process_again_timeout_failure_to_proceed`:
   - configure the datalogger and run the loop once so it takes ownership;
   - stop running the loop; send `ResetDatalogger`;
   - call `h.process(step)` with steps that stay below the timeout: no response is queued (`data_to_send() == 0`);
   - step past `REQUEST_MAX_PROCESS_TIME_US * 10`: a response is queued with code `FailureToProceed` and empty payload;
   - after popping it, the handler accepts a new request (e.g. GetStatus answers `OK`).
2. `test_process_again_completes_before_timeout`: same setup, but run the loop between main ticks; the request completes with `OK` before the timeout.

## Acceptance

- New tests pass. All checks listed in [00_README.md](00_README.md) pass.
