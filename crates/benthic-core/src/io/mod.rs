//! Serialization formats and format detection.

pub mod csv;
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
    /// A one-dive-per-row CSV export.
    Csv,
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
        } else if lower.ends_with(".csv") || lower.ends_with(".tsv") {
            Some(Format::Csv)
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
            Format::Csv => csv::parse_str(contents),
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
        Some(Format::Csv) => csv::parse_str(contents),
        None => Err(crate::Error::Parse {
            what: "dive log format",
            value: contents.chars().take(64).collect(),
        }),
    }
}

/// Parse a dive log, preferring the format implied by the filename and falling
/// back to content sniffing.
pub fn parse_named(name: &str, contents: &str) -> Result<DiveLog> {
    match Format::from_extension(name) {
        Some(format) => format.parse(contents),
        None => parse_auto(contents),
    }
}
