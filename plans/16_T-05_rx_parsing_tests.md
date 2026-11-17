# Plan 16: T-05, port the missing RX parsing tests

## Gap

The C++ suite `TestRxParsing` (`../scrutiny-embedded/test/protocol/test_protocol_rx_parsing.cpp`) has 12 tests. Rust covers part of it in `tests/comm_handler.rs` (`test_rx_framing_valid`, `test_rx_bad_crc_no_request`, `test_rx_invalid_command_bit`, `test_rx_timeout_resets_fsm`, `test_rx_overflow`, `test_rx_chunked_delivery`). Six C++ tests have no equivalent.

## Context

- New file `tests/protocol_rx_parsing.rs` (or extend `tests/comm_handler.rs`; prefer a new file to mirror the C++ layout), testing `CommHandler` directly. Copy helpers from `tests/comm_handler.rs`.
- `receive_data(data, rx_buf, tb)` feeds bytes; `request_received()` / `get_request()` expose the result as `(command_id, subfunction_id, data_length)`; the payload is in the caller's `rx_buf`.
- Without a session, only Discover and Connect requests are accepted. Either connect first, or use Discover/Connect frames, as the C++ tests do.
- RX timeout: `RX_TIMEOUT_100NS`; overflow recovery happens after the RX timeout.

## What to do

Port with the same frames and expectations:

1. `test_rx_zero_len_all_in_one` (`TestRx_ZeroLen_AllInOne`).
2. `test_rx_zero_len_byte_per_byte` (`TestRx_ZeroLen_BytePerByte`).
3. `test_rx_non_zero_len_split_in_len` (`TestRx_NonZeroLen_SplitInLen`): the 2-byte length field arrives in two calls.
4. `test_rx_discover_2_parts` (`TestRx_Discover_2_parts`).
5. `test_rx_use_all_buffer` (`TestRx_UseAllBuffer`): payload length equals the RX buffer size → accepted.
6. `test_rx_overflow_restore_after_delay` (`TestRx_OverflowRestoreAfterDelay`): after an overflow, a valid frame is accepted only once the RX timeout has passed.

Also check whether `TestRx_NonZeroLen_AllInOne`, `TestRx_NonZeroLen_BytePerByte` and `TestRx_ReceiveResponseBit_Ignore` are fully covered by the existing Rust tests; if any assertion is missing, add it.

## Acceptance

- New tests pass. All checks listed in [00_README.md](00_README.md) pass.
