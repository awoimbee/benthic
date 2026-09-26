//! GPX import.
//!
//! GPX files carry waypoints and tracks, which are useful as dive sites. We
//! turn each waypoint into a named [`DiveSite`], and each track's first point
//! into a site named after the track (when it has a name).

use super::xml::{parse_document, Node};
use crate::model::DiveSite;
use crate::units::Location;
use crate::{Error, Result};

/// Parse a GPX document into dive sites.
pub fn parse_sites(xml: &str) -> Result<Vec<DiveSite>> {
    let mut root = parse_document(xml)?;
    if root.name != "gpx" {
        match root.children.iter().position(|c| c.name == "gpx") {
            Some(pos) => root = root.children.swap_remove(pos),
            None => {
                return Err(Error::Parse {
                    what: "GPX root element",
                    value: root.name,
                })
            }
        }
    }

    let mut sites = Vec::new();
    let mut next_uuid = 1u32;

    for waypoint in root.children_named("wpt") {
        if let Some(location) = location_of(waypoint) {
            sites.push(DiveSite {
                uuid: next_uuid,
                name: child_text(waypoint, "name"),
                location: Some(location),
                ..Default::default()
            });
            next_uuid += 1;
        }
    }

    for track in root.children_named("trk") {
        let first = track
            .children_named("trkseg")
            .flat_map(|segment| segment.children_named("trkpt"))
            .next();
        if let Some(point) = first {
            if let Some(location) = location_of(point) {
                sites.push(DiveSite {
                    uuid: next_uuid,
                    name: child_text(track, "name"),
                    location: Some(location),
                    ..Default::default()
                });
                next_uuid += 1;
            }
        }
    }

    Ok(sites)
}

fn child_text(node: &Node, name: &str) -> String {
    node.child(name)
        .map(|child| child.text_trimmed().to_string())
        .unwrap_or_default()
}

fn location_of(node: &Node) -> Option<Location> {
    let lat: f64 = node.attr("lat")?.parse().ok()?;
    let lon: f64 = node.attr("lon")?.parse().ok()?;
    let location = Location::new(lat, lon);
    location.is_valid().then_some(location)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GPX: &str = r#"<?xml version="1.0"?>
<gpx version="1.1">
  <wpt lat="28.572100" lon="34.536700"><name>Blue Hole</name></wpt>
  <wpt lat="28.610000" lon="34.540000"><name>Lighthouse</name></wpt>
  <trk>
    <name>Shore entry</name>
    <trkseg>
      <trkpt lat="28.500000" lon="34.500000"><time>2024-05-12T08:00:00Z</time></trkpt>
      <trkpt lat="28.501000" lon="34.501000" />
    </trkseg>
  </trk>
</gpx>
"#;

    #[test]
    fn imports_waypoints_and_tracks() {
        let sites = parse_sites(GPX).unwrap();
        assert_eq!(sites.len(), 3);
        assert_eq!(sites[0].name, "Blue Hole");
        assert_eq!(sites[0].location, Some(Location::new(28.5721, 34.5367)));
        assert_eq!(sites[2].name, "Shore entry");
        assert_eq!(sites[2].location, Some(Location::new(28.5, 34.5)));
        // uuids are unique.
        let mut ids: Vec<u32> = sites.iter().map(|s| s.uuid).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), sites.len());
    }

    #[test]
    fn rejects_non_gpx() {
        assert!(parse_sites("<divelog></divelog>").is_err());
    }
}
