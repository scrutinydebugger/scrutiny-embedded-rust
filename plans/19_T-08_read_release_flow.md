# Plan 19: T-08, test the read-with-release flow

## Gap

Rust reads a completed acquisition only after the owning loop has released the datalogger (accepted difference D-03, see `ACCEPTED_DIFFERENCES.md`). The existing read tests go through the `send_and_recv_with_release` helper, which hides the intermediate states. No test asserts the flow explicitly.

## Context

- Logic in `process_datalog_control` ([src/main_handler.rs:1138-1207](../src/main_handler.rs#L1138-L1207)):
  - `GetAcquisitionMetadata` / `ReadAcquisition` with state `AcquisitionCompleted` and an owner: set `reset_on_release = false`, request a release, return `ProcessAgain`.
  - When `OwnershipReleased` arrives with `reset_on_release == false`, the datalogger is **not** reset, so the data is preserved; `reset_on_release` goes back to `true` for the next cycle.
  - After the release there is no owner, so `ArmTrigger` / `DisarmTrigger` return `FailureToProceed`.
- Server sequence this must support (`../scrutiny-main/scrutiny/server/device/submodules/datalogging_poller.py`): Configure → Arm → GetStatus polling → GetAcquisitionMetadata → ReadAcquisition chunks → ResetDatalogger. The server never re-arms after reading.
- Helpers in `tests/comm_datalog_control.rs`: `make_handler_with_loops`, `send_configure`, the acquisition-completion helper (loop that ends with `panic!("Acquisition did not complete within 500 iterations")`), `send_and_recv_with_release`.

## What to do

Add to `tests/comm_datalog_control.rs`:

1. `test_read_release_flow_explicit`:
   - configure, arm, run the loop until the acquisition completes;
   - send `GetAcquisitionMetadata` and call `h.process(0)` once: no response yet (`data_to_send() == 0`), `datalogging_ownership_taken()` still true;
   - run the loop once (`process_with_time(0)`, or `process(0)` if plan 09 is not done yet) then `h.process(0)`: response `OK` with metadata, ownership no longer taken;
   - read all `ReadAcquisition` chunks; check the CRC, and check the data matches what was logged (the data was not reset by the release);
   - `GetStatus` still reports `AcquisitionCompleted`.
2. `test_arm_after_read_fails`: after step 1, `ArmTrigger` returns `FailureToProceed`.
3. `test_full_server_sequence`: Configure → Arm → GetStatus until complete → GetAcquisitionMetadata → ReadAcquisition until finished → ResetDatalogger → Configure again → Arm again works. This mirrors the server poller.
4. `test_release_without_read_resets`: Configure while a loop owns the datalogger (release with the default `reset_on_release = true`) → the datalogger is reset and reconfigured, so the earlier `reset_on_release = false` path does not leak into later releases.

## Acceptance

- New tests pass. All checks listed in [00_README.md](00_README.md) pass.
