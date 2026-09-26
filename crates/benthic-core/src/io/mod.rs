//! Serialization formats and format detection.

pub mod gpx;
pub mod json;
pub mod ssrf;
pub(crate) mod xml;

use crate::model::DiveLog;
use crate::Result;

/// A dive-log file format supported by benthic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// benthic's native, lossless JSON format.
    BenthicJson,
    /// Subsurface's XML format (`.ssrf`, `Subsurface <version> XML`).
    SubsurfaceXml,
    /// GPS Exchange Format (`.gpx`), imported as dive sites.
    Gpx,
}

impl Format {
    /// Detect the format from a file's contents.
    pub fn detect(contents: &str) -> Option<Self> {
        let trimmed = contents.trim_start_matches('\u{feff}').trim_start();
        if trimmed.starts_with('{') {
            return Some(Format::BenthicJson);
        }
        if trimmed.starts_with('<')
            && (trimmed.contains("divelog") || trimmed.contains("subsurface"))
        {
            return Some(Format::SubsurfaceXml);
        }
        if trimmed.starts_with('<') && trimmed.contains("<gpx") {
            return Some(Format::Gpx);
        }
        None
    }

    /// Detect the format from a filename extension.
    pub fn from_extension(path: &str) -> Option<Self> {
        let lower = path.to_ascii_lowercase();
        if lower.ends_with(".json") || lower.ends_with(".benthic") {
            Some(Format::BenthicJson)
        } else if lower.ends_with(".ssrf") || lower.ends_with(".xml") {
            Some(Format::SubsurfaceXml)
        } else if lower.ends_with(".gpx") {
            Some(Format::Gpx)
        } else {
            None
        }
    }

    pub fn parse(self, contents: &str) -> Result<DiveLog> {
        match self {
            Format::BenthicJson => json::from_str(contents),
            Format::SubsurfaceXml => ssrf::parse_str(contents),
            Format::Gpx => {
                let mut log = DiveLog::new();
                log.sites = gpx::parse_sites(contents)?;
                Ok(log)
            }
        }
    }
}

/// Parse a dive log, guessing the format from its contents.
pub fn parse_auto(contents: &str) -> Result<DiveLog> {
    match Format::detect(contents) {
        Some(Format::BenthicJson) => json::from_str(contents),
        Some(Format::SubsurfaceXml) => ssrf::parse_str(contents),
        Some(Format::Gpx) => {
            let mut log = DiveLog::new();
            log.sites = gpx::parse_sites(contents)?;
            Ok(log)
        }
        None => Err(crate::Error::Parse {
            what: "dive log format",
            value: contents.chars().take(64).collect(),
        }),
    }
}
