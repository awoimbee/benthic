//! The dive-log domain model.
//!
//! The model is deliberately plain data: no pointers, no Qt-style ownership
//! tricks. Dives reference trips and sites by id, which keeps the structure
//! trivially serializable and makes undo/redo and diffing feasible later.

use serde::{Deserialize, Serialize};

use crate::gas::GasMix;
use crate::units::*;

// ---------------------------------------------------------------------------
// Enumerations
// ---------------------------------------------------------------------------

/// Which breathing mode a dive (or dive computer) used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Divemode {
    #[default]
    OpenCircuit,
    Ccr,
    Pscr,
    Freedive,
}

impl Divemode {
    pub fn is_rebreather(self) -> bool {
        matches!(self, Divemode::Ccr | Divemode::Pscr)
    }

    pub fn label(self) -> &'static str {
        match self {
            Divemode::OpenCircuit => "OC",
            Divemode::Ccr => "CCR",
            Divemode::Pscr => "pSCR",
            Divemode::Freedive => "Freedive",
        }
    }
}

/// How a cylinder was used during the dive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CylinderUse {
    #[default]
    OcGas,
    Diluent,
    Oxygen,
    NotUsed,
    TravelOc,
}

impl CylinderUse {
    /// Every variant, in display order.
    pub const ALL: [CylinderUse; 5] = [
        CylinderUse::OcGas,
        CylinderUse::Diluent,
        CylinderUse::Oxygen,
        CylinderUse::NotUsed,
        CylinderUse::TravelOc,
    ];

    /// Position of this variant within [`CylinderUse::ALL`].
    pub fn index(self) -> usize {
        match self {
            CylinderUse::OcGas => 0,
            CylinderUse::Diluent => 1,
            CylinderUse::Oxygen => 2,
            CylinderUse::NotUsed => 3,
            CylinderUse::TravelOc => 4,
        }
    }

    /// Inverse of [`CylinderUse::index`]; falls back to [`CylinderUse::OcGas`].
    pub fn from_index(index: usize) -> Self {
        Self::ALL.get(index).copied().unwrap_or_default()
    }

    pub fn label(self) -> &'static str {
        match self {
            CylinderUse::OcGas => "OC gas",
            CylinderUse::Diluent => "Diluent",
            CylinderUse::Oxygen => "Oxygen",
            CylinderUse::NotUsed => "Not used",
            CylinderUse::TravelOc => "Travel gas",
        }
    }
}

// ---------------------------------------------------------------------------
// Samples, events and dive computers
// ---------------------------------------------------------------------------

/// One pressure reading from a specific (wireless) tank sensor.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SensorPressure {
    pub sensor: i16,
    pub pressure: Pressure,
}

/// One sample of the dive profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Sample {
    /// Elapsed time since the start of the dive.
    pub time: Duration,
    pub depth: Depth,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<Temperature>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pressures: Vec<SensorPressure>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setpoint: Option<O2Pressure>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub o2_sensors: Vec<O2Pressure>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ndl: Option<Duration>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tts: Option<Duration>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rbt: Option<Duration>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_time: Option<Duration>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_depth: Option<Depth>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cns: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heartbeat: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bearing: Option<Bearing>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub in_deco: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub manually_entered: bool,
}

/// A notable moment during the dive (gas switch, deco stop, marker, alarm...).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Event {
    pub time: Duration,
    /// Display name, e.g. "gaschange", "deco", "Marker".
    pub name: String,
    pub flags: i32,
    pub value: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub divemode: Option<Divemode>,
    /// Gas switch payload: `(cylinder_index, gas)`. `-1` means "match by mix".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gas: Option<(i32, GasMix)>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
}

impl Event {
    pub fn is_gas_change(&self) -> bool {
        self.gas.is_some()
    }

    pub fn is_divemode_change(&self) -> bool {
        self.name == "modechange"
    }
}

/// Extra key/value metadata reported by a dive computer.
pub type ExtraData = (String, String);

/// A single dive computer's recording of a dive.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct DiveComputer {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub device_id: u32,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub dive_id: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub firmware: Option<String>,
    #[serde(default)]
    pub divemode: Divemode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_depth: Option<Depth>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mean_depth: Option<Depth>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub air_temp: Option<Temperature>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub water_temp: Option<Temperature>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface_pressure: Option<Pressure>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub salinity: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<Duration>,
    /// Seconds east of UTC as reported by the computer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timezone_offset: Option<i32>,
    #[serde(default, skip_serializing_if = "is_zero_u8")]
    pub no_o2_sensors: u8,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub samples: Vec<Sample>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<Event>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_data: Vec<ExtraData>,
}

impl DiveComputer {
    /// Duration inferred from the last sample if not explicitly stored.
    pub fn duration_or_last_sample(&self) -> Option<Duration> {
        self.duration
            .or_else(|| self.samples.last().map(|s| s.time))
    }
}

// ---------------------------------------------------------------------------
// Equipment
// ---------------------------------------------------------------------------

/// A cylinder used on a dive.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Cylinder {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<Volume>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_pressure: Option<Pressure>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default)]
    pub gas: GasMix,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_pressure: Option<Pressure>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_pressure: Option<Pressure>,
    #[serde(default)]
    pub use_: CylinderUse,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<Depth>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub manually_added: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub bestmix_o2: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub bestmix_he: bool,
}

/// A weighting system (belt, integrated, ...).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct WeightSystem {
    #[serde(default)]
    pub weight: Weight,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub auto_filled: bool,
}

impl WeightSystem {
    pub fn new(weight: Weight, description: impl Into<String>) -> Self {
        Self {
            weight,
            description: description.into(),
            auto_filled: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Media
// ---------------------------------------------------------------------------

/// A photo or video associated with a dive.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Picture {
    pub filename: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<Duration>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<Location>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
}

// ---------------------------------------------------------------------------
// Dives, trips and sites
// ---------------------------------------------------------------------------

/// A single dive.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Dive {
    /// Stable internal identifier.
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub id: u32,
    /// User-visible dive number.
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub number: i32,
    /// Start time, seconds since the Unix epoch (UTC).
    pub when: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trip_id: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub site_id: Option<u32>,

    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub notes: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub diveguide: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub buddy: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub suit: String,

    /// Ratings are 0 (unset) to 5.
    #[serde(default, skip_serializing_if = "is_zero_u8")]
    pub rating: u8,
    #[serde(default, skip_serializing_if = "is_zero_u8")]
    pub visibility: u8,
    #[serde(default, skip_serializing_if = "is_zero_u8")]
    pub wavesize: u8,
    #[serde(default, skip_serializing_if = "is_zero_u8")]
    pub current: u8,
    #[serde(default, skip_serializing_if = "is_zero_u8")]
    pub surge: u8,
    #[serde(default, skip_serializing_if = "is_zero_u8")]
    pub chill: u8,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub salinity: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub air_temp: Option<Temperature>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub water_temp: Option<Temperature>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<Duration>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_depth: Option<Depth>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mean_depth: Option<Depth>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface_pressure: Option<Pressure>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cylinders: Vec<Cylinder>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub weights: Vec<WeightSystem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub computers: Vec<DiveComputer>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pictures: Vec<Picture>,
    /// Exclude this dive from automatic trip grouping.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub no_trip: bool,
}

impl Dive {
    /// A new, empty, manually-entered dive starting at `when`.
    pub fn manual(when: Timestamp) -> Self {
        Self {
            when,
            computers: vec![DiveComputer {
                model: "Manually entered".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    /// The primary dive computer, if any.
    pub fn primary_computer(&self) -> Option<&DiveComputer> {
        self.computers.first()
    }

    /// A specific dive computer by index.
    pub fn computer(&self, index: usize) -> Option<&DiveComputer> {
        self.computers.get(index)
    }

    /// Best-effort duration: explicit value, else the longest computer.
    pub fn duration(&self) -> Option<Duration> {
        self.duration.or_else(|| {
            self.computers
                .iter()
                .filter_map(DiveComputer::duration_or_last_sample)
                .max_by_key(|d| d.seconds)
        })
    }

    /// Best-effort maximum depth: explicit value, else the deepest computer.
    pub fn max_depth(&self) -> Option<Depth> {
        self.max_depth.or_else(|| {
            self.computers
                .iter()
                .filter_map(|dc| dc.max_depth)
                .max_by_key(|d| d.mm)
        })
    }

    pub fn total_weight(&self) -> Weight {
        Weight::new(self.weights.iter().map(|w| w.weight.grams).sum())
    }
}

/// A group of dives that happened together (a dive trip/holiday).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct DiveTrip {
    pub id: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date: Option<Timestamp>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub location: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub notes: String,
    /// Created automatically by the trip grouping algorithm.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub auto_generated: bool,
}

/// A named dive location, optionally with coordinates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct DiveSite {
    pub uuid: u32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<Location>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub notes: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ocean: Option<String>,
}

/// A known dive computer (for autodetection and naming).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Device {
    pub model: String,
    #[serde(default)]
    pub device_id: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nickname: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub firmware: Option<String>,
}

// ---------------------------------------------------------------------------
// The log itself
// ---------------------------------------------------------------------------

/// The root document: everything benthic knows about the diver's history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiveLog {
    pub version: u32,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub autogroup: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dives: Vec<Dive>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trips: Vec<DiveTrip>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sites: Vec<DiveSite>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub devices: Vec<Device>,
}

impl Default for DiveLog {
    fn default() -> Self {
        Self {
            version: 1,
            autogroup: true,
            dives: Vec::new(),
            trips: Vec::new(),
            sites: Vec::new(),
            devices: Vec::new(),
        }
    }
}

impl DiveLog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.dives.is_empty()
    }

    pub fn dive_by_id(&self, id: u32) -> Option<&Dive> {
        self.dives.iter().find(|d| d.id == id)
    }

    pub fn site_by_uuid(&self, uuid: u32) -> Option<&DiveSite> {
        self.sites.iter().find(|s| s.uuid == uuid)
    }

    pub fn trip_by_id(&self, id: u32) -> Option<&DiveTrip> {
        self.trips.iter().find(|t| t.id == id)
    }

    /// Allocate an id that is not yet in use.
    pub fn next_id(&self) -> u32 {
        self.dives.iter().map(|d| d.id).max().unwrap_or(0) + 1
    }

    /// Allocate a free dive-site uuid.
    pub fn next_site_uuid(&self) -> u32 {
        self.sites.iter().map(|s| s.uuid).max().unwrap_or(0) + 1
    }

    /// Allocate a free trip id.
    pub fn next_trip_id(&self) -> u32 {
        self.trips.iter().map(|t| t.id).max().unwrap_or(0) + 1
    }

    // -- Primitive mutations -------------------------------------------------
    //
    // These are the building blocks used by `history::Command`. They are
    // deliberately dumb; use the command stack for anything user-visible so
    // that it stays undoable.

    /// Insert a dive at `index` (clamped to the end).
    pub fn insert_dive(&mut self, index: usize, dive: Dive) {
        let index = index.min(self.dives.len());
        self.dives.insert(index, dive);
    }

    /// Remove a dive by id, returning it and its former position.
    pub fn take_dive(&mut self, id: u32) -> Option<(usize, Dive)> {
        let index = self.dives.iter().position(|d| d.id == id)?;
        Some((index, self.dives.remove(index)))
    }

    /// Remove a dive by id, discarding it.
    pub fn remove_dive(&mut self, id: u32) {
        self.dives.retain(|d| d.id != id);
    }

    /// Replace the dive with the same id. Returns whether a dive was found.
    pub fn replace_dive(&mut self, dive: Dive) -> bool {
        match self.dives.iter_mut().find(|d| d.id == dive.id) {
            Some(slot) => {
                *slot = dive;
                true
            }
            None => false,
        }
    }

    /// Replace the trip with the same id. Returns whether a trip was found.
    pub fn replace_trip(&mut self, trip: DiveTrip) -> bool {
        match self.trips.iter_mut().find(|t| t.id == trip.id) {
            Some(slot) => {
                *slot = trip;
                true
            }
            None => false,
        }
    }

    /// Replace the site with the same uuid. Returns whether a site was found.
    pub fn replace_site(&mut self, site: DiveSite) -> bool {
        match self.sites.iter_mut().find(|s| s.uuid == site.uuid) {
            Some(slot) => {
                *slot = site;
                true
            }
            None => false,
        }
    }

    /// The human name of a dive's site, if set.
    pub fn site_name_of(&self, dive: &Dive) -> Option<&str> {
        dive.site_id
            .and_then(|id| self.site_by_uuid(id))
            .map(|s| s.name.as_str())
            .filter(|name| !name.is_empty())
    }

    /// Find groups of dive sites that look like duplicates of each other.
    ///
    /// Two sites are duplicates when their names match (ignoring case and
    /// extra whitespace) and their coordinates are compatible: either one is
    /// missing, or they are within `radius_m` of each other. Returns groups of
    /// site uuids, each of size >= 2.
    pub fn mergeable_site_groups(&self, radius_m: f64) -> Vec<Vec<u32>> {
        fn normalize(name: &str) -> String {
            name.split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase()
        }
        let compatible = |a: Option<Location>, b: Option<Location>| match (a, b) {
            (Some(a), Some(b)) => a.distance_m(b) <= radius_m,
            _ => true,
        };

        let mut assigned = vec![false; self.sites.len()];
        let mut groups: Vec<Vec<u32>> = Vec::new();
        for i in 0..self.sites.len() {
            if assigned[i] {
                continue;
            }
            let site = &self.sites[i];
            let name = normalize(&site.name);
            if name.is_empty() {
                continue;
            }
            let mut group = vec![site.uuid];
            for (j, flag) in assigned.iter_mut().enumerate().skip(i + 1) {
                if *flag {
                    continue;
                }
                let other = &self.sites[j];
                if normalize(&other.name) == name && compatible(site.location, other.location) {
                    *flag = true;
                    group.push(other.uuid);
                }
            }
            if group.len() > 1 {
                assigned[i] = true;
                groups.push(group);
            }
        }
        groups
    }

    /// Merge every group of duplicate sites, returning the number of sites
    /// removed. Dives referencing a removed site are repointed at the kept one;
    /// missing fields on the kept site are filled from the first duplicate that
    /// has them. Useful after importing legacy logs that repeat inline site
    /// names.
    pub fn merge_duplicate_sites(&mut self, radius_m: f64) -> usize {
        let groups = self.mergeable_site_groups(radius_m);
        let mut removed = 0;
        for group in groups {
            let keep_uuid = group[0];
            let remove: Vec<u32> = group[1..].to_vec();

            // Fill in missing details on the kept site.
            let extras: Vec<DiveSite> = remove
                .iter()
                .filter_map(|uuid| self.site_by_uuid(*uuid).cloned())
                .collect();
            if let Some(keep) = self.sites.iter_mut().find(|s| s.uuid == keep_uuid) {
                for extra in &extras {
                    if keep.location.is_none() {
                        keep.location = extra.location;
                    }
                    if keep.description.is_empty() {
                        keep.description = extra.description.clone();
                    }
                    if keep.notes.is_empty() {
                        keep.notes = extra.notes.clone();
                    }
                    if keep.country.is_none() {
                        keep.country = extra.country.clone();
                    }
                    if keep.ocean.is_none() {
                        keep.ocean = extra.ocean.clone();
                    }
                }
            }

            for dive in &mut self.dives {
                if dive.site_id.is_some_and(|id| remove.contains(&id)) {
                    dive.site_id = Some(keep_uuid);
                }
            }
            let before = self.sites.len();
            self.sites.retain(|s| !remove.contains(&s.uuid));
            removed += before - self.sites.len();
        }
        removed
    }

    /// Merge `remove_id` into `keep_id`: move its dives across, fill in missing
    /// fields on the kept trip, and delete the removed trip. Returns whether
    /// anything changed.
    pub fn merge_trips(&mut self, keep_id: u32, remove_id: u32) -> bool {
        if keep_id == remove_id || self.trip_by_id(keep_id).is_none() {
            return false;
        }
        let Some(remove) = self.trip_by_id(remove_id).cloned() else {
            return false;
        };

        for dive in &mut self.dives {
            if dive.trip_id == Some(remove_id) {
                dive.trip_id = Some(keep_id);
            }
        }
        if let Some(keep) = self.trips.iter_mut().find(|t| t.id == keep_id) {
            if keep.location.is_empty() {
                keep.location = remove.location.clone();
            }
            if keep.notes.is_empty() {
                keep.notes = remove.notes.clone();
            }
            if keep.date.is_none() {
                keep.date = remove.date;
            }
        }
        self.trips.retain(|t| t.id != remove_id);
        true
    }

    /// Remove all automatically-generated trips and clear the trip links of
    /// dives that referenced them. Manual trips are left untouched.
    pub fn clear_auto_trips(&mut self) {
        let auto_ids: std::collections::HashSet<u32> = self
            .trips
            .iter()
            .filter(|t| t.auto_generated)
            .map(|t| t.id)
            .collect();
        self.trips.retain(|t| !t.auto_generated);
        for dive in &mut self.dives {
            if dive.trip_id.is_some_and(|id| auto_ids.contains(&id)) {
                dive.trip_id = None;
            }
        }
    }

    /// Automatically group consecutive dives into trips.
    ///
    /// Dives are grouped when the gap to the next dive is at most
    /// `max_gap_days` and neither dive is explicitly excluded (`no_trip`) or
    /// already assigned to a manual trip. Previously auto-generated trips are
    /// discarded first, so this is idempotent. Returns the number of trips
    /// created. Does nothing when `autogroup` is disabled.
    pub fn autogroup_trips(&mut self, max_gap_days: i64) -> usize {
        if !self.autogroup {
            return 0;
        }

        // Discard previous automatic grouping.
        self.clear_auto_trips();

        // (id, when, blocked) sorted by time.
        let manual_trip_ids: std::collections::HashSet<u32> = self
            .trips
            .iter()
            .filter(|t| !t.auto_generated)
            .map(|t| t.id)
            .collect();
        let mut ordered: Vec<(u32, Timestamp, bool)> = self
            .dives
            .iter()
            .map(|d| {
                let manual = d.trip_id.is_some_and(|id| manual_trip_ids.contains(&id));
                (d.id, d.when, d.no_trip || manual)
            })
            .collect();
        ordered.sort_by_key(|(_, when, _)| *when);

        let max_gap = max_gap_days * 86_400;
        let mut created = 0;
        let mut i = 0;
        while i < ordered.len() {
            if ordered[i].2 {
                i += 1;
                continue;
            }
            let start = i;
            let mut end = i + 1;
            while end < ordered.len()
                && !ordered[end].2
                && ordered[end].1 - ordered[end - 1].1 <= max_gap
            {
                end += 1;
            }
            if end - start >= 2 {
                let first_id = ordered[start].0;
                let id = self.next_trip_id();
                let (date, location) = {
                    let dive = self.dive_by_id(first_id).expect("dive exists");
                    (dive.when, self.site_name_of(dive).unwrap_or("").to_string())
                };
                self.trips.push(DiveTrip {
                    id,
                    date: Some(date),
                    location,
                    auto_generated: true,
                    ..Default::default()
                });
                for (dive_id, _, _) in &ordered[start..end] {
                    if let Some(dive) = self.dives.iter_mut().find(|d| d.id == *dive_id) {
                        dive.trip_id = Some(id);
                    }
                }
                created += 1;
            }
            i = end;
        }
        created
    }

    /// Merge another log into this one, renumbering ids to avoid collisions.
    pub fn merge(&mut self, other: DiveLog) {
        use std::collections::HashMap;

        let mut site_map = HashMap::new();
        for mut site in other.sites {
            let new = self.next_site_uuid();
            site_map.insert(site.uuid, new);
            site.uuid = new;
            self.sites.push(site);
        }

        let mut trip_map = HashMap::new();
        for mut trip in other.trips {
            let new = self.next_trip_id();
            trip_map.insert(trip.id, new);
            trip.id = new;
            self.trips.push(trip);
        }

        for mut dive in other.dives {
            dive.site_id = dive.site_id.and_then(|s| site_map.get(&s).copied());
            dive.trip_id = dive.trip_id.and_then(|t| trip_map.get(&t).copied());
            dive.id = self.next_id();
            self.dives.push(dive);
        }
    }

    /// Dives sorted oldest-first, which is how the UI shows them.
    pub fn dives_sorted(&self) -> Vec<&Dive> {
        let mut dives: Vec<&Dive> = self.dives.iter().collect();
        dives.sort_by_key(|d| d.when);
        dives
    }

    /// Re-derive per-dive summary fields from the dive computers.
    pub fn fixup_all(&mut self) {
        for dive in &mut self.dives {
            if dive.duration.is_none() {
                dive.duration = dive
                    .computers
                    .iter()
                    .filter_map(DiveComputer::duration_or_last_sample)
                    .max_by_key(|d| d.seconds);
            }
            if dive.max_depth.is_none() {
                dive.max_depth = dive
                    .computers
                    .iter()
                    .filter_map(|dc| dc.max_depth)
                    .max_by_key(|d| d.mm);
            }
        }
    }
}

fn is_zero_u32(v: &u32) -> bool {
    *v == 0
}
fn is_zero_u8(v: &u8) -> bool {
    *v == 0
}
fn is_zero_i32(v: &i32) -> bool {
    *v == 0
}
