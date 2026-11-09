#![cfg_attr(not(feature = "std"), no_std)]
#![deny(warnings)]
#![warn(clippy::all, clippy::pedantic, clippy::nursery)]
#![allow(clippy::struct_excessive_bools)]
#![allow(clippy::too_many_lines)]

pub mod build_config;
pub mod protocol;
pub mod timebase;
pub mod tools;
pub mod types;
