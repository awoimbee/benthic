//! The crate error type and libdivecomputer status handling.

use std::os::raw::c_int;

use crate::ffi::*;

/// Anything that can go wrong talking to libdivecomputer.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("libdivecomputer error: {message} (status {status})")]
    Status { status: i32, message: &'static str },
    #[error("unknown dive computer: {vendor} {product}")]
    UnknownDevice { vendor: String, product: String },
    #[error("no matching {transport} device found")]
    NoDevice { transport: &'static str },
    #[error("download cancelled")]
    Cancelled,
    #[error("{0}")]
    Message(String),
}

pub(crate) fn check(status: c_int) -> Result<(), Error> {
    if status == DC_STATUS_SUCCESS {
        Ok(())
    } else {
        Err(Error::Status {
            status,
            message: status_message(status),
        })
    }
}

fn status_message(status: c_int) -> &'static str {
    match status {
        DC_STATUS_DONE => "done",
        DC_STATUS_UNSUPPORTED => "unsupported",
        DC_STATUS_INVALIDARGS => "invalid arguments",
        DC_STATUS_NOMEMORY => "out of memory",
        DC_STATUS_NODEVICE => "device not found",
        DC_STATUS_NOACCESS => "permission denied",
        DC_STATUS_IO => "I/O error",
        DC_STATUS_TIMEOUT => "timeout",
        DC_STATUS_PROTOCOL => "protocol error",
        DC_STATUS_DATAFORMAT => "unexpected data format",
        DC_STATUS_CANCELLED => "cancelled",
        _ => "unknown error",
    }
}
