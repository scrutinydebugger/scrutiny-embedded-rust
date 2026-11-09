// Build-time configuration. Values are set via environment variables at compile
// time (e.g. SCRUTINY_COMM_RX_TIMEOUT_US=100000 cargo build).
// Defaults mirror the C++ reference implementation.
include!(concat!(env!("OUT_DIR"), "/config.rs"));
