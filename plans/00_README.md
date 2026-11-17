# Implementation plans: gap closure with the C++ reference

Each file in this folder is a standalone plan for one fresh agent. Item IDs come from [GAP_ANALYSIS.md](../GAP_ANALYSIS.md) and the decisions come from [GAP_ANALYSIS_ACTION.md](../GAP_ANALYSIS_ACTION.md).

Related files:
- [CPP_IMPROVEMENTS.md](CPP_IMPROVEMENTS.md): changes to make later in the C++ repo (`../scrutiny-embedded`). No plan here touches the C++ repo.
- [../ACCEPTED_DIFFERENCES.md](../ACCEPTED_DIFFERENCES.md): differences accepted on purpose. Read it before "fixing" a difference with the C++ code.

## Rules for every plan

- Read `CLAUDE.md` (project) first. Key points: `no_std`, no heap, no `unwrap()` / `expect()` / `panic!` / `unreachable!` in library code, function pointers instead of closures, one-line doc comment on every public item, follow the C++ module architecture.
- C++ reference sources: `../scrutiny-embedded/lib/` (library) and `../scrutiny-embedded/test/` (tests). When a plan says "mirror C++", read the named C++ function first.
- Integration tests live in `tests/*.rs`. They use local helpers (`make_request`, `send_and_recv`, `pop_response`, `connect`). Copy the helpers from an existing file such as `tests/comm_control.rs` rather than inventing new ones. The datalogging tests in `tests/comm_datalog_control.rs` have extra helpers (`make_handler_with_loops`, `send_configure`, `send_and_recv_with_release`).
- Python-style rule from the user also applies to Rust tests: describe a test with a `//` comment, not a doc comment.
- Environment: the `cargo` on PATH is a snap package that fails in the agent sandbox. Prefix every shell command that uses cargo with `export PATH="$HOME/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin:$PATH";` (the shell does not keep environment variables between calls).
- Done means all of these pass:
  - `cargo build`
  - `cargo build --no-default-features`
  - `cargo build --no-default-features --features datalogging`
  - `cargo test --all-features`
  - `cargo clippy --all-features -- -D warnings`
- Do not run git commands that modify the branch, history or working tree.

## Suggested order

Plans touching the same files are ordered to limit conflicts. Plans with no dependency can run in parallel. Note: plan 09 runs **before** plan 08 (the table is in execution order).

Loop API vocabulary after plan 09: `FixedFreqLoopHandler` (`process()`, `process_with_time(dt)`), `VariableFreqLoopHandler` (`process_with_time(dt)` only), both wrapping the shared core `LoopHandler` that is registered with `Config::set_loops(&'static [&'static LoopHandler])`. Plans written before that change refer to `loop.process(dt)`; read it as `process_with_time(dt)` once plan 09 is done.

`MainHandler` must not be moved after its first `process()` call (loops hold the datalogger address). It is meant to be a global instance living for the whole program. This is an accepted constraint, see `ACCEPTED_DIFFERENCES.md`; tests must respect it.

| # | Plan | Depends on | Main files |
|---|---|---|---|
| 01 | [F-01 + T-01: UserCommand](01_F-01_user_command.md) | – | `main_handler.rs`, new `tests/comm_user_command.rs` |
| 02 | [F-02 + T-10: RPV type check at init](02_F-02_rpv_type_check.md) | – | `main_handler.rs`, `tools.rs`, `datalogging/datalogger.rs` |
| 03 | [F-03: reject boolean16/32/64](03_F-03_reject_sized_booleans.md) | – | `tests/comm_datalog_control.rs` |
| 04 | [D-06: empty ReadRPV request](04_D-06_read_rpv_empty_request.md) | – | `protocol/codec_v1_0.rs`, `tests/comm_memory_control.rs` |
| 05 | [Q-01: remove `unreachable!`](05_Q-01_remove_unreachable.md) | – | `protocol/comm_handler.rs` |
| 06 | [A-04: getter names](06_A-04_getter_names.md) | – | `main_handler.rs` |
| 07 | [F-06: LoopHandler without datalogging](07_F-06_loop_handler_without_datalogging.md) | – | `loop_handler.rs`, `main_handler.rs` |
| 09 | [F-07: split into fixed / variable loop types](09_F-07_fixed_freq_process.md) | 07 | `loop_handler.rs`, `config.rs`, `main_handler.rs`, all tests |
| 08 | [D-01: datalogging enabled by default](08_D-01_datalogging_default_enabled.md) | 07, 09 | `loop_handler.rs`, all tests |
| 10 | [F-08: clear loop IPC at init](10_F-08_loop_init_clear.md) | 07, 09 | `loop_handler.rs`, `ipc.rs`, `main_handler.rs` |
| 11 | [F-09: extensible IPC flag](11_F-09_ipc_flag_extensibility.md) | 07, 10 | `loop_handler.rs`, `ipc.rs` |
| 12 | [A-06: datalogger buffer API](12_A-06_datalogger_buffer_api.md) | – | `config.rs`, tests |
| 13 | [T-02: RPV tests](13_T-02_rpv_tests.md) | 04 | `tests/comm_memory_control.rs` |
| 14 | [T-03: CommHandler tests](14_T-03_comm_handler_tests.md) | – | `tests/comm_handler.rs` |
| 15 | [T-04: TX parsing tests](15_T-04_tx_parsing_tests.md) | 05 | new `tests/protocol_tx_parsing.rs` |
| 16 | [T-05: RX parsing tests](16_T-05_rx_parsing_tests.md) | – | new `tests/protocol_rx_parsing.rs` |
| 17 | [T-06: misc tests](17_T-06_misc_tests.md) | – | `tests/` |
| 18 | [T-07: ProcessAgain timeout](18_T-07_process_again_timeout.md) | – | `tests/comm_datalog_control.rs` |
| 19 | [T-08: read-with-release flow](19_T-08_read_release_flow.md) | – | `tests/comm_datalog_control.rs` |
| 20 | [T-09: datalogging disabled on loop](20_T-09_datalogging_disabled.md) | 08 | `tests/comm_datalog_control.rs`, `tests/get_info.rs` |
| 21 | [T-11: threaded acquisition](21_T-11_threaded_acquisition.md) | 08 | new `tests/datalogging_threaded.rs` |

## Items with no plan

Decided as "leave as is", "ignore" or "report to C++": F-04, F-05, F-10, F-11, F-12, D-02, D-03, D-04, D-05, D-07, D-08, D-09, D-10, D-11, A-01, A-02, A-03, A-05, A-07, Q-02, T-12. See the two related files above.
