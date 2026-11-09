use std::env;
use std::fmt::Write as FmtWrite;
use std::fs;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-env-changed=SCRUTINY_COMM_RX_TIMEOUT_US");
    println!("cargo:rerun-if-env-changed=SCRUTINY_COMM_HEARTBEAT_TIMEOUT_US");
    println!("cargo:rerun-if-env-changed=SCRUTINY_REQUEST_MAX_PROCESS_TIME_US");
    println!("cargo:rerun-if-env-changed=SCRUTINY_DATALOGGING_MAX_SIGNAL");
    println!("cargo:rerun-if-env-changed=SCRUTINY_PROTOCOL_VERSION_MAJOR");
    println!("cargo:rerun-if-env-changed=SCRUTINY_PROTOCOL_VERSION_MINOR");
    println!("cargo:rerun-if-env-changed=SCRUTINY_DATALOGGING_ENCODING");

    let rx_timeout = read_u32("SCRUTINY_COMM_RX_TIMEOUT_US", 50_000);
    let hb_timeout = read_u32("SCRUTINY_COMM_HEARTBEAT_TIMEOUT_US", 5_000_000);
    let max_process_time = read_u32("SCRUTINY_REQUEST_MAX_PROCESS_TIME_US", 100_000);
    let max_signal = read_u32("SCRUTINY_DATALOGGING_MAX_SIGNAL", 32);
    let proto_major = read_u32("SCRUTINY_PROTOCOL_VERSION_MAJOR", 1);
    let proto_minor = read_u32("SCRUTINY_PROTOCOL_VERSION_MINOR", 0);
    let dl_encoding = read_datalogging_encoding("SCRUTINY_DATALOGGING_ENCODING", 0);

    let mut out = String::new();
    writeln!(out, "pub const COMM_RX_TIMEOUT_US: u32 = {rx_timeout};").unwrap();
    writeln!(out, "pub const COMM_HEARTBEAT_TIMEOUT_US: u32 = {hb_timeout};").unwrap();
    writeln!(out, "pub const REQUEST_MAX_PROCESS_TIME_US: u32 = {max_process_time};").unwrap();
    writeln!(out, "pub const PROTOCOL_VERSION_MAJOR: u8 = {proto_major} as u8;").unwrap();
    writeln!(out, "pub const PROTOCOL_VERSION_MINOR: u8 = {proto_minor} as u8;").unwrap();
    writeln!(out, "#[cfg(feature = \"datalogging\")]").unwrap();
    writeln!(out, "pub const DATALOGGING_MAX_SIGNAL: usize = {max_signal} as usize;").unwrap();
    writeln!(out, "#[cfg(feature = \"datalogging\")]").unwrap();
    writeln!(out, "pub const DATALOGGING_ENCODING: u32 = {dl_encoding};").unwrap();

    let out_dir = env::var("OUT_DIR").unwrap();
    fs::write(Path::new(&out_dir).join("config.rs"), out).unwrap();
}

fn read_u32(var: &str, default: u32) -> u32 {
    match env::var(var) {
        Err(_) => default,
        Ok(s) => s.parse::<u32>().unwrap_or_else(|_| {
            panic!("{var} must be a positive integer, got {s:?}")
        }),
    }
}

fn read_datalogging_encoding(var: &str, default: u32) -> u32 {
    match env::var(var) {
        Err(_) => default,
        Ok(s) => match s.to_lowercase().as_str() {
            "raw" | "0" => 0,
            _ => panic!("{var} must be \"raw\" or 0, got {s:?}"),
        },
    }
}
