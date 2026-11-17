# Plan 05: Q-01, remove `unreachable!()` from `CommHandler::pop_data`

## Gap

[src/protocol/comm_handler.rs:327](../src/protocol/comm_handler.rs#L327) uses `unreachable!()` in the header-byte `match`. This is a panic in library code, forbidden by `CLAUDE.md` and `REQUIREMENTS.md`. Decision: remove it and replace it with a silent cancel (go to an error state).

## Context

- `pop_data(&mut self, out, tx_buf, tb) -> u16` streams the 5 header bytes (`cmd|0x80`, subfunction, response code, length high, length low), then the payload, then the 4 CRC bytes. The `match self.nbytes_sent` covers 0–4 and the loop condition guarantees `nbytes_sent < 5`, so the `_` arm is impossible in practice.
- Existing ways to abort: `reset_tx()` (clears TX state, returns to `Idle` if transmitting) and `wait_next_request(tb)` (clears RX). `pop_data` already calls both when the frame is fully sent.
- `TxError` has `None`, `Overflow`, `Busy`, `Disabled`. There is no "internal error" variant.

## What to do

1. Replace the `_ => unreachable!()` arm with a silent cancel:
   - stop the transmission: `self.reset_tx(); self.wait_next_request(tb);`
   - return the number of bytes already written to `out` (`i`).
   - Since the value is produced inside a `match` expression, restructure the loop slightly (e.g. compute an `Option<u8>` and handle `None` by cancelling), keeping the code simple.
2. Do not add a new `TxError` variant unless needed to make the cancel observable; if you add one, document it and keep the C++ variants untouched in meaning. Prefer no new variant.
3. Search the rest of `src/` (outside `#[cfg(test)]`) for `unreachable!`, `panic!`, `unwrap()`, `expect(`, `todo!`, `unimplemented!`. At the time of the analysis only this one existed; fix any new one the same way and list it in your report.

## Tests

The branch is not reachable from the public API, so no direct test is possible. Make sure the existing TX tests in `tests/comm_handler.rs` still pass. Plan 15 (T-04) adds wider TX coverage.

## Acceptance

- No panic macro left in library code. All checks listed in [00_README.md](00_README.md) pass.
