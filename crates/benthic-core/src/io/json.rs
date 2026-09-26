//! benthic's native JSON format.
//!
//! This is the canonical, lossless on-disk format. It is simply the serde
//! representation of [`DiveLog`](crate::model::DiveLog), pretty-printed so it
//! can be diffed in git.

use crate::model::DiveLog;
use crate::Result;

/// Serialize a dive log to pretty-printed JSON.
pub fn to_string(log: &DiveLog) -> Result<String> {
    Ok(serde_json::to_string_pretty(log)?)
}

/// Serialize a dive log to compact JSON.
pub fn to_compact_string(log: &DiveLog) -> Result<String> {
    Ok(serde_json::to_string(log)?)
}

/// Parse a dive log from JSON.
pub fn from_str(s: &str) -> Result<DiveLog> {
    Ok(serde_json::from_str(s)?)
}
