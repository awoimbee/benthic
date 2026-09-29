//! libdivecomputer integration for benthic.
//!
//! This crate is native-only. On `wasm32` it compiles to an empty crate, so
//! the web build never pulls in the C library or any transport code.
//!
//! The library itself is vendored under `vendor/libdivecomputer` and built by
//! `build.rs`; see the crate README for the required tools.
#![cfg(not(target_arch = "wasm32"))]

#[cfg(feature = "native")]
pub mod ble;

/// Stub for builds without the BlueZ backend, so `device.rs` compiles
/// everywhere. Every entry point reports that BLE is unavailable.
#[cfg(not(feature = "native"))]
pub mod ble {

    use std::time::Duration;

    use crate::ffi::{dc_context_t, dc_iostream_t};
    use crate::Error;

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct BleDevice {
        pub address: String,
        pub name: String,
    }

    pub struct BleConnection;

    impl BleConnection {
        pub fn connect(_address: &str) -> Result<Self, Error> {
            Err(Error::Message("BLE support is not compiled in".to_string()))
        }

        /// # Safety
        /// Never returns an iostream; present only to satisfy the call sites.
        pub unsafe fn open_iostream(
            &self,
            _context: *mut dc_context_t,
        ) -> Result<*mut dc_iostream_t, Error> {
            Err(Error::Message("BLE support is not compiled in".to_string()))
        }
    }

    pub fn scan(_timeout: Duration) -> Result<Vec<BleDevice>, Error> {
        Err(Error::Message("BLE support is not compiled in".to_string()))
    }
}
mod device;
mod error;
mod ffi;
mod parse;

pub use device::{
    download, scan, DeviceEvent, DeviceId, DeviceInfo, DiscoveredDevice, Download, DownloadedDive,
};
pub use error::Error;
pub use parse::{descriptors, parse_dump, DeviceDescriptor, Transport};
