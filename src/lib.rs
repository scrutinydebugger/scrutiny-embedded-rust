#![cfg_attr(not(feature = "std"), no_std)]
#![deny(warnings)]
#![warn(clippy::all, clippy::pedantic, clippy::nursery)]
#![allow(clippy::struct_excessive_bools)]
#![allow(clippy::too_many_lines)]

pub mod build_config;
pub mod config;
#[cfg(feature = "datalogging")]
pub mod datalogging;
pub mod ipc;
pub mod loop_handler;
pub mod main_handler;
pub mod protocol;
pub mod software_id;
pub mod timebase;
pub mod tools;
pub mod types;
