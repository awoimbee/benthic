//! benthic-core — the platform-agnostic heart of benthic.
//!
//! This crate knows nothing about UI, browsers or the filesystem. It owns:
//!
//! * the strongly-typed dive-log domain model ([`model`]),
//! * explicit, integer-based physical [`units`] to avoid floating-point drift,
//! * [`gas`] mixing helpers,
//! * serialization: the native JSON format and Subsurface-compatible XML
//!   ([`io`]).
//!
//! Everything above this crate (storage, import/export UX, rendering) is
//! provided by the application crate.

pub mod filter;
pub mod gas;
pub mod history;
pub mod io;
pub mod model;
pub mod units;

pub use filter::DiveFilter;
pub use gas::GasMix;
pub use history::{Command, History};
pub use model::*;
pub use units::*;

/// Convenience result alias used throughout the crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors that can occur while parsing or serializing dive logs.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("XML error: {0}")]
    Xml(#[from] quick_xml::Error),

    #[error("XML attribute error: {0}")]
    XmlAttr(#[from] quick_xml::events::attributes::AttrError),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("could not parse {what}: {value:?}")]
    Parse { what: &'static str, value: String },

    #[error("invalid date or time: {0}")]
    DateTime(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}
