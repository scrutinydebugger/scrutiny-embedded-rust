# Gap analysis: scrutiny-embedded (C++) vs scrutiny-embedded-rust

Reference for implementation planning. Item IDs (`F-xx`, `D-xx`, `A-xx`, `Q-xx`, `T-xx`) are stable so a plan can refer to them.

- Analysis date: 2026-11-17
- C++ reference: `../scrutiny-embedded/lib`
- Server reference: `../scrutiny-main/scrutiny/server`
- Method: code reading only. `cargo` could not run in the analysis sandbox, so nothing was built or executed. Items marked **[unverified]** are assumptions.

## Intentional design changes (not gaps)

- Timebases are passed by function call rather than by reference at init.
- `MainHandler` owns the comm buffers.
- `MainHandler` / `CommHandler` interaction reworked for lifetime constraints.

## Wire compatibility summary

For every command the Rust port implements, the byte layout matches the C++ codec: GetInfo, CommControl, MemoryControl, DataLogControl, framing, CRC, heartbeat and session logic. The only server-visible break is **F-01** (UserCommand).

---

## 1. Missing features

| ID | Item | Impact | Location |
|---|---|---|---|
| F-01 | **UserCommand (0x04) not implemented.** The dispatch always returns `UnsupportedFeature`, but `GetSupportedFeatures` still reports `user_command = true` when a callback is set. The server is told the feature exists, then gets a NACK. `UserCommandCallback` and `Config::set_user_command_callback` already exist. C++ behaviour: call the callback with (subfn, request data, response buffer, &len, tx_buffer_size); `Overflow` if returned length > TX buffer size; `UnsupportedFeature` if no callback. | Server-visible break | `src/main_handler.rs:542`, `:164` |
| F-02 | **No RPV type validation at init.** C++ `check_config()` disables the handler if any RPV type fails `tools::is_supported_type` (rejects float16, cfloat, sizes > 8, etc.). Rust only checks buffer sizes. Side effect: Rust ReadRPV returns `FailureToProceed` and WriteRPV returns `InvalidRequest` on 0-size types, where C++ never reaches that path. | Config validation | `src/main_handler.rs:157-217` |
| F-03 | **`VariableType` lacks `Boolean16`, `Boolean32`, `Boolean64`.** A datalog Configure using them is rejected with `InvalidRequest` at decode. Low impact for Rust targets (bool is 1 byte, server should send `boolean8`). | Low | `src/types.rs:28` |
| F-04 | **Datalogger not reachable by the user.** C++ `MainHandler::datalogger()` allows `force_trigger()`. In Rust `DataLoggerCell` is `pub(crate)`, so `DataLogger::force_trigger` is unreachable. Needs a thread-safe path (e.g. an IPC message or a flag). | API | `src/datalogging/mod.rs:23` |
| F-05 | **Public `MainHandler` API missing:** `get_rpv`, `rpv_exists`, `get_rpv_type`, `get_rpv_read_callback`, `fetch_variable`, `fetch_variable_bitfield`, `comm()`, `get_config()`. | API | `src/main_handler.rs` |
| F-06 | **`LoopHandler::process()` only exists with the `datalogging` feature**, so user code calling it fails to compile without the feature. The timebase is also feature-gated. | API / build | `src/loop_handler.rs:198` |
| F-07 | **No fixed-frequency `process()` without argument** (C++ `FixedFrequencyLoopHandler::process()` uses the configured timestep). | API | `src/loop_handler.rs` |
| F-08 | **No `LoopHandler::init()` equivalent.** C++ clears both IPC channels when `MainHandler::init` runs. | Robustness | `src/loop_handler.rs` |
| F-09 | **IPC flag not pluggable.** `AtomicFlag` trait exists, but `LoopHandler` hard-codes `StdAtomicFlag`. C++ ships AVR, TriCore, TI C28, x86 variants. | Portability | `src/loop_handler.rs:99`, `src/ipc.rs` |
| F-10 | **No compile-time check `DATALOGGING_MAX_SIGNAL <= 254`** (C++ `static_assert`). Rust saturates to 255 in GetSetup. | Low | `build.rs`, `src/main_handler.rs:1015` |
| F-11 | **No `SCRUTINY_DATALOGGING_BUFFER_32BITS` equivalent.** Acceptable: Rust uses `usize`, protocol fields are `u32`. | None | `build.rs` |
| F-12 | **Probably out of scope:** C wrapper (`cwrapper/`), `CHAR_BIT == 16` targets, `tools::make_address_range` helpers. | Scope decision | — |

---

## 2. Behaviour differences

### 2.1 Visible to the server or the integrator

| ID | Item | Location |
|---|---|---|
| D-01 | **Loop datalogging default.** C++ `m_support_datalogging = true`; Rust `datalogging_allowed = false` with opt-in `with_datalogging()`. Without opt-in the server sees no datalogging (feature bit 0, loop attribute 0). | `src/loop_handler.rs:119`, `:139` |
| D-02 | **Datalogging buffer set but no loop allows datalogging.** C++ still answers `GetSetup`. Rust does not create the datalogger and returns `UnsupportedFeature` for every DataLogControl subfunction. The server does not query in that case (feature bit is 0 in both). | `src/main_handler.rs:174-186`, `:546-549` |
| D-03 | **Acquisition read flow.** Rust requests ownership release from the loop (returns `ProcessAgain` until released) before `GetAcquisitionMetadata` / `ReadAcquisition`; C++ reads while the loop owns it. Consequences: Arm/Disarm after a read returns `FailureToProceed`; if the loop does not release within `REQUEST_MAX_PROCESS_TIME_US`, the request becomes `FailureToProceed`. **Compatible with the server**: `datalogging_poller.py` always runs Configure → Arm → Status → Metadata → Read → Reset and never re-arms after a read. | `src/main_handler.rs:1138-1207` |
| D-04 | **`DataAcquired` message.** Rust sets cached state to `AcquisitionCompleted`; C++ ignores the message and waits for the next status update. Harmless. | `src/main_handler.rs:431` |
| D-05 | **Firmware ID tagging [unverified].** `SOFTWARE_ID` is a `#[no_mangle] pub static`. The compiler may constant-fold reads of an immutable static into immediates; if so, a binary patched by `scrutiny tag-firmware-id` would still report the placeholder. C++ avoids this with an `extern const` in a separate TU. To verify: inspect release LLVM IR / disassembly of `encode_response_get_info_software_id` and `encode_response_comm_discover`. | `src/software_id.rs:3` |

### 2.2 Edge cases (not sent by the server)

| ID | Item | Location |
|---|---|---|
| D-06 | Empty ReadRPV request: C++ `InvalidRequest`, Rust `OK` with no data. | `src/protocol/codec_v1_0.rs:753` |
| D-07 | GetLoopDefinition with wrong length: Rust `InvalidRequest`; C++ ignores the decode error and uses an uninitialised `loop_id`. Rust is better; keep. | `src/main_handler.rs:754` |
| D-08 | Configure with unknown condition or VariableType: Rust rejects at decode, before the `loop_id` check. A request wrong in both ways gets `InvalidRequest` instead of C++ `FailureToProceed`. | `src/protocol/codec_v1_0.rs:1000`, `:1035` |
| D-09 | VarBit operand with `bitsize == 0`: rejected at configure in Rust; C++ accepts and fails at fetch time. | `src/datalogging/datalogger.rs:205` |
| D-10 | ReadRPV: Rust checks overflow before calling the read callback; C++ calls it first. | `src/protocol/codec_v1_0.rs:772` |
| D-11 | Bitfield fetch with boolean type and bitsize > 8: C++ outputs boolean16/32 via the uint path; Rust returns failure (only `Boolean8` handled). Linked to F-03. | `src/datalogging/datalogger.rs:650` |

### 2.3 API differences (intentional or acceptable)

| ID | Item | Location |
|---|---|---|
| A-01 | `RpvReadCallback` and trigger callback receive the loop name (`Option<&'static str>`) instead of `LoopHandler*`. Loop names are not guaranteed unique. | `src/config.rs:11`, `:24` |
| A-02 | `RpvWriteCallback` has no `caller` argument (C++ has one, always null from the main handler). | `src/config.rs:14` |
| A-03 | `MainHandler::new` returns `None` when buffers are missing (C++ `init` returns `ERROR`, object stays usable but disabled). | `src/main_handler.rs:157` |
| A-04 | Getter names differ: `datalogging_state` vs `get_datalogger_state`, `datalogging_in_error` vs `datalogging_error`. | `src/main_handler.rs:346`, `:364` |
| A-05 | Session counter is per instance (C++ static). | `src/protocol/comm_handler.rs:34` |
| A-06 | `Config::set_datalogger_buffer` takes a raw pointer + length (`unsafe`). Could take `&'static mut [u8]`. | `src/config.rs:245` |
| A-07 | `UserCommandCallback` signature is idiomatic (`fn(u8, &[u8], &mut [u8], &mut u16)`); response max length is the slice length. | `src/config.rs:18` |

---

## 3. Code-rule issues found during the analysis

| ID | Item | Location |
|---|---|---|
| Q-01 | `unreachable!()` in library code breaks the no-panic rule. | `src/protocol/comm_handler.rs:327` |
| Q-02 | Display name (64 B) and loop name (32 B) are truncated on a byte boundary and can split a UTF-8 character. The server decodes with strict `.decode('utf8')` (`scrutiny-main/scrutiny/server/protocol/protocol.py:963`, `:1120`). C++ has the same flaw; Rust can truncate on a char boundary (`str::is_char_boundary`). | `src/protocol/codec_v1_0.rs:96-98`, `:372-375` |

---

## 4. Tests

### 4.1 C++ tests with no Rust equivalent

| ID | C++ suite | Missing tests |
|---|---|---|
| T-01 | `TestUserCommand` | `TestCommandCalled`, `TestResponseOverflow`, `TestNoCallback` (depends on F-01) |
| T-02 | `TestMemoryControlRPV` | `TestReadMultipleRPVEachType`, `TestReadRPVResponseOverflow`, `TestWriteAllTypes`, `TestWriteRPVResponseOverflow`, `TestWriteRPVResponseFullNoOverflow` |
| T-03 | `TestCommHandler` | ~~`TestHeartbeatTimeoutExpire`, `TestConnectDisconnectBehaviour`~~ **Correction:** both were already ported in `tests/comm_handler.rs` (`test_heartbeat_timeout_expire`, `test_connect_disconnect`); missed by the analysis. Extra checks added in commit `226910c`. |
| T-04 | `TestTxParsing` | `TestReadAllData`, `TestReadBytePerByte`, `TestReadMoreThanAvailable`, `TestSendsOverflow`, `TestSendsFullBuffer`, `TestSendsOverflowRestart` |
| T-05 | `TestRxParsing` | `TestRx_ZeroLen_AllInOne`, `TestRx_ZeroLen_BytePerByte`, `TestRx_NonZeroLen_SplitInLen`, `TestRx_Discover_2_parts`, `TestRx_UseAllBuffer`, `TestRx_OverflowRestoreAfterDelay` |
| T-06 | Misc | `TestIPC.CheckWithThread`, `TestVariableFetching.RandomFetch`, `TestDataLoggingTypes.AnyTypeCompareCast` |

Not applicable: `TestCodecs` (Rust uses `to_be_bytes`), `TestCWrapper`, `TestFetchOperands.TestBadOperandType` (prevented by the type system), `*NotMultipleOfCharDenied` (`CHAR_BIT` 16).

Already at parity: trigger conditions, datalogger, raw encoder, GetInfo, CommControl, MemoryControl, DataLogControl.

### 4.2 Tests needed for Rust-specific behaviour (no C++ equivalent)

| ID | Test |
|---|---|
| T-07 | `ProcessAgain` that exceeds `REQUEST_MAX_PROCESS_TIME_US` produces `FailureToProceed`. |
| T-08 | Read while a loop owns the datalogger: metadata/read return `ProcessAgain`, then `OK`; acquisition data preserved; Arm afterwards returns `FailureToProceed` (D-03). |
| T-09 | Loop with datalogging off + datalogging buffer configured: feature bit 0 and `UnsupportedFeature` on DataLogControl (D-01, D-02). |
| T-10 | RPV type validation at init (after F-02). |
| T-11 | End-to-end acquisition with the loop running on another thread. |
| T-12 | UTF-8 truncation of display/loop names on a char boundary (after Q-02). |

---

## 5. Suggested priority

1. F-01 + T-01 (only server-visible break).
2. F-02 + T-10.
3. D-01 (decide the default for loop datalogging).
4. D-05 (verify firmware ID tagging).
5. Q-01, Q-02.
6. Remaining API gaps (F-04 to F-09) and tests (T-02 to T-09, T-11).
