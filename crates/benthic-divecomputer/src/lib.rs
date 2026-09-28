//! libdivecomputer integration for benthic.
//!
//! This crate is native-only. On `wasm32` it compiles to an empty crate, so
//! the web build never pulls in the C library or any transport code.
//!
//! The library itself is vendored under `vendor/libdivecomputer` and built by
//! `build.rs`; see the crate README for the required tools.
#![cfg(not(target_arch = "wasm32"))]

mod device;
mod error;
mod ffi;
mod parse;

pub use device::{
    download, scan, DeviceEvent, DeviceId, DeviceInfo, DiscoveredDevice, Download, DownloadedDive,
};
pub use error::Error;
pub use parse::{descriptors, parse_dump, DeviceDescriptor, Transport};
