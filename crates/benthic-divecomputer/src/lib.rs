//! libdivecomputer integration for benthic.
//!
//! This crate is native-only. On `wasm32` it compiles to an empty crate, so
//! the web build never pulls in the C library or any transport code.
//!
//! The library itself is vendored under `vendor/libdivecomputer` and built by
//! `build.rs`; see the crate README for the required tools.
#![cfg(not(target_arch = "wasm32"))]

mod ffi;
mod parse;

pub use parse::{descriptors, parse_dump, DeviceDescriptor, Error, Transport};
