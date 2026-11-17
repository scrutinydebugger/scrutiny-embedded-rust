# Plan 01: F-01 + T-01, implement the UserCommand service

## Gap

The `UserCommand` command (`CommandId::UserCommand = 0x04`) is not implemented. `MainHandler::dispatch` maps it to `UnsupportedFeature` ([src/main_handler.rs:542](../src/main_handler.rs#L542)). But `GetInfo/GetSupportedFeatures` reports `user_command = true` when a callback is set ([src/main_handler.rs:164](../src/main_handler.rs#L164)), so the server is told the feature exists and then gets a NACK. This is the only server-visible break in the port.

## Context

- C++ reference: `MainHandler::process_user_command` in `../scrutiny-embedded/lib/src/scrutiny_main_handler.cpp` (around line 1335):
  - If no callback is set: `UnsupportedFeature`.
  - Otherwise call `callback(subfunction_id, request_data, request_data_length, response_data, &response_data_length, tx_buffer_size)`.
  - If `response_data_length > tx_buffer_size`: `Overflow`. Otherwise set the response data length and return `OK`.
  - In `process_request`, any code other than `OK` forces `data_length = 0`. The Rust `process()` already does this.
- Rust callback type already exists in [src/config.rs:18](../src/config.rs#L18): `pub type UserCommandCallback = fn(u8, &[u8], &mut [u8], &mut u16);` (subfunction, request data, response buffer, response length out). The maximum response length is the response slice length; do not change the signature (accepted as A-07).
- `MainHandler::new` currently keeps only `user_command_configured: bool`. It must keep the callback itself.
- Dispatch pattern: `dispatch()` calls associated functions (`Self::process_xxx`) and passes the fields it needs separately, so `self.rx_buf` (request data) and `self.tx_buf` (response data) can be borrowed at the same time. Follow the same pattern.

## What to do

1. In `MainHandler`, replace `user_command_configured: bool` with `user_command_callback: Option<UserCommandCallback>`. Update `GetSupportedFeatures` to use `self.user_command_callback.is_some()`.
2. Add `fn process_user_command(comm: &mut CommHandler, tx_buf: &mut [u8], callback: Option<UserCommandCallback>, subfn_id: u8, data: &[u8]) -> ResponseCode`:
   - `None` → `UnsupportedFeature`.
   - `Some(cb)` → `let mut len: u16 = 0; cb(subfn_id, data, tx_buf, &mut len);`
   - If `usize::from(len) > tx_buf.len()` → `Overflow`. Otherwise set `comm.active_response_mut().data_length = len` and return `Ok`.
3. In `dispatch()`, route `Ok(CommandId::UserCommand)` to it. Keep `Err(())` → `UnsupportedFeature`.
4. Doc comment on the new function and keep the existing doc on `UserCommandCallback` accurate (mention that the response buffer length is the maximum response length).

## Tests (T-01)

New file `tests/comm_user_command.rs`, built like `tests/comm_control.rs` (copy its helpers). Port the three tests from `../scrutiny-embedded/test/commands/test_user_command.cpp`:

- `test_command_called`: callback records its inputs in statics (tests may use `std`, e.g. `static` + `Mutex` or atomics, because callbacks are plain `fn` pointers). Request: command 4, subfunction `0xAA`, payload `[0x12, 0x34, 0x56]`. Callback writes `[0x11, 0x22, 0x33, 0x44]` and sets length 4. Check:
  - the callback saw subfunction `0xAA`, request length 3 and the 3 payload bytes;
  - the response buffer it received has length equal to the TX buffer size;
  - the full response frame is `[0x84, 0xAA, 0x00, 0x00, 0x04, 0x11, 0x22, 0x33, 0x44]` followed by the CRC.
- `test_response_overflow`: callback sets length to `response.len() + 1`. Expect response code `Overflow` with an empty payload.
- `test_no_callback`: no callback set. Expect `UnsupportedFeature`.
- Add `test_supported_features_bit`: with a callback, `GetSupportedFeatures` has bit `0x20` set; without, it is clear. (A similar test exists in `tests/get_info.rs`; keep it, this one is optional if it duplicates.)

Each test must `connect()` first, since requests other than Discover/Connect are dropped without a session.

## Acceptance

- The three ported tests pass.
- All checks listed in [00_README.md](00_README.md) pass.
