# Plan 15: T-04, port the TX parsing tests

## Gap

The C++ suite `TestTxParsing` (`../scrutiny-embedded/test/protocol/test_protocol_tx_parsing.cpp`) has 7 tests; Rust covers only consecutive sends and partial pops (`test_consecutive_send`, `test_pop_data_partial_reads` in `tests/comm_handler.rs`).

## Context

- Depends on plan 05 (Q-01), which edits `CommHandler::pop_data`.
- New file `tests/protocol_tx_parsing.rs`, testing `CommHandler` directly like `tests/comm_handler.rs` (copy its helpers).
- To send a response with `CommHandler`: set fields through `active_response_mut()` (command, subfunction, response code, `data_length`), write the payload into the caller's TX buffer, call `send_response(&tx_buf)`, then read bytes with `pop_data(&mut out, &tx_buf, &tb)`. `send_response` requires `state == Idle`, so make sure no RX is in progress (see how the existing TX tests do it; they may need a session or a received request first).
- Frame: `cmd|0x80`, subfunction, code, length (2 bytes, big-endian), payload, CRC32 (4 bytes, big-endian) over the header + payload.

## What to do

Port each C++ test with the same data and expectations:

1. `test_read_all_data` (`TestReadAllData`): pop the whole frame in one call; check bytes and CRC; `data_to_send()` becomes 0 and the handler is ready for the next request.
2. `test_read_byte_per_byte` (`TestReadBytePerByte`).
3. `test_read_by_chunk` (`TestReadByChunk`); keep or merge with the existing partial-read test, but cover the C++ chunk sizes.
4. `test_read_more_than_available` (`TestReadMoreThanAvailable`): output buffer bigger than the frame → returns only the frame size.
5. `test_sends_overflow` (`TestSendsOverflow`): `data_length` greater than the TX buffer → `send_response` returns `false`, `get_tx_error() == TxError::Overflow`, nothing to pop.
6. `test_sends_full_buffer` (`TestSendsFullBuffer`): payload exactly the TX buffer size → OK.
7. `test_sends_overflow_restart` (`TestSendsOverflowRestart`): after an overflow, a valid response can still be sent.

## Acceptance

- New tests pass. All checks listed in [00_README.md](00_README.md) pass.
