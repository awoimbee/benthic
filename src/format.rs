//! Presentation helpers shared by components.

use benthic_core::units::format_duration;
use benthic_core::{Dive, DiveLog, Preferences};

/// A human-friendly title for a dive: its site name when known.
pub fn dive_title(dive: &Dive, log: &DiveLog) -> String {
    if let Some(name) = dive
        .site_id
        .and_then(|id| log.site_by_uuid(id))
        .map(|s| s.name.clone())
    {
        if !name.is_empty() {
            return name;
        }
    }
    if dive.number != 0 {
        format!("Dive #{}", dive.number)
    } else {
        "Dive".to_string()
    }
}

/// A one-line summary of a dive.
pub fn dive_subtitle(dive: &Dive, prefs: &Preferences) -> String {
    let duration = dive
        .duration()
        .map(format_duration)
        .unwrap_or_else(|| "—".into());
    let depth = dive
        .max_depth()
        .map(|d| prefs.depth(d))
        .unwrap_or_else(|| "—".into());
    let date = prefs.timestamp(dive.when);
    format!("{date}  ·  {duration}  ·  {depth}")
}
