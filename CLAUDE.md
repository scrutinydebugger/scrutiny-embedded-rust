# CLAUDE.md — scrutiny-embedded-rust

## Project

Rust port of the scrutiny-embedded C++ instrumentation library.

- C++ reference: https://github.com/scrutinydebuggerscrutiny-embedded/lib/
- Documentation: https://scrutinydebugger.com/

## Build & Test

```bash
# Build
cargo build 

# Build with cherry picked features
cargo build --no-default-features --features datalogging

# Test (requires std feature for the test harness)
cargo test --all-features 

# Lint
cargo clippy --all-features -- -D warnings
```

## Architecture

- `no_std` by default. `std` is a feature used only to enable the test harness.
- No heap allocation. No `Box`, `Vec`, `String`, or any allocator type.
- No `unwrap()`, no `expect()` in library code. Use `Result` or explicit match.
- No `panic!` in library code except inside `#[cfg(test)]`.
- Caller-provided buffers everywhere (mirrors the C++ design).

## Cargo Features

| Feature | C++ equivalent | Description |
|---|---|---|
| `std` | n/a | Enables test harness. Never used in production. |
| `datalogging` | `SCRUTINY_ENABLE_DATALOGGING` | Oscilloscope-like data capture |
| `support_64bits` | `SCRUTINY_SUPPORT_64BITS` | Adds u64/i64/f64 variable types |
| `protected_regions` | `SCRUTINY_SUPPORT_PROTECTED_REGIONS` | Forbidden/read-only address ranges |


## Code Style

- Follow the C++ module boundaries — one Rust module per C++ source file pair.
- Time unit throughout: 100ns increments (`u32`), type alias `Timediff`.
- Port enums as Rust enums, not integer constants.
- Port C++ callback pointers as function pointers (`fn(...) -> ...`), not closures.
- All public types and functions must have a one-line doc comment.
- Unit tests go inline at the bottom of each module file (`#[cfg(test)]`).
