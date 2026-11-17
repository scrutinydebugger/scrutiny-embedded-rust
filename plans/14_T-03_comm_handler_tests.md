# Plan 14: T-03, port missing CommHandler tests

## Gap

Two tests from `../scrutiny-embedded/test/protocol/test_comm_handler.cpp` have no Rust equivalent: `TestHeartbeatTimeoutExpire` and `TestConnectDisconnectBehaviour`.

## Context

- Target file: `tests/comm_handler.rs`. It tests `CommHandler` directly (not through `MainHandler`), with helpers `make_request`, `make_expected_response`, `make_comm_buffers`.
- Rust API difference (intentional): every time-dependent `CommHandler` method takes `tb: &Timebase` explicitly, and the caller owns the buffers (`receive_data(data, rx_buf, tb)`, `pop_data(out, tx_buf, tb)`, `send_response(tx_buf)`).
- Heartbeat timeout: `HEARTBEAT_TIMEOUT_100NS` (from `build_config::COMM_HEARTBEAT_TIMEOUT_US * 10`). `CommHandler::process(tb)` resets the session when it expires.
- `heartbeat(challenge, tb)` rejects a repeated challenge after the first heartbeat.

## What to do

1. `test_heartbeat_timeout_expire`: mirror the C++ test. Connect; step a local `Timebase` to just under the timeout, call `process` → still connected; send a heartbeat with a new challenge → timer refreshed; step past the timeout without a heartbeat → `process` disconnects (`is_connected() == false`, session id 0). Use the exact time steps from the C++ test when possible.
2. `test_connect_disconnect_behaviour`: mirror the C++ test. Check that a second connect while connected fails, that disconnect resets the session, that the session ID changes between sessions (counter increments from the seed), and that non-Discover/Connect requests are dropped while disconnected.

## Acceptance

- Two new tests pass. All checks listed in [00_README.md](00_README.md) pass.
