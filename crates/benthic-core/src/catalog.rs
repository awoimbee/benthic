//! Global lists of people and gear.
//!
//! Dives keep their own free-text fields (`buddy`, `diveguide`, `suit`,
//! cylinder and weight descriptions) so that Subsurface XML round-trips stay
//! lossless. The catalog adds identity and metadata on top: it is the curated
//! list the UI offers when editing a dive, and it works out usage by matching
//! those per-dive names. Gear kinds without a dedicated dive field (BCD,
//! regulator, torch, ...) are linked explicitly through [`Dive::gear_ids`].

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::model::{Dive, DiveLog};
use crate::units::Timestamp;

/// Bump when a new seeding pass must backfill existing logs. Version 1 covers
/// people, suits, bottles, weights and dive computers.
///
/// Version 2 also copies each site's stored position onto its dives, since
/// site positions are now derived from dive locations.
pub const CATALOG_VERSION: u32 = 2;

/// Someone who can appear on a dive: a buddy, a dive guide, ...
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Person {
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub id: u32,
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub notes: String,
}

/// The broad kind of a piece of gear. The first three match fields every dive
/// already has, so their usage can be derived without an explicit link.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
#[serde(rename_all = "snake_case")]
pub enum GearKind {
    #[default]
    Wetsuit,
    Bottle,
    WeightBelt,
    Bcd,
    Regulator,
    DiveComputer,
    Mask,
    Fins,
    Torch,
    Camera,
    Other,
}

impl GearKind {
    /// Every kind, in display order.
    pub const ALL: [GearKind; 11] = [
        GearKind::Wetsuit,
        GearKind::Bottle,
        GearKind::WeightBelt,
        GearKind::Bcd,
        GearKind::Regulator,
        GearKind::DiveComputer,
        GearKind::Mask,
        GearKind::Fins,
        GearKind::Torch,
        GearKind::Camera,
        GearKind::Other,
    ];

    /// Whether dives already carry this kind in a dedicated field (suit,
    /// cylinders, weights or the dive-computer record), rather than needing
    /// an explicit [`Dive::gear_ids`] link.
    pub fn has_dedicated_field(self) -> bool {
        matches!(
            self,
            GearKind::Wetsuit | GearKind::Bottle | GearKind::WeightBelt | GearKind::DiveComputer
        )
    }

    /// A stable English label, used for storage-adjacent output. The UI
    /// translates through [`crate::format`]'s string table instead.
    pub fn label(self) -> &'static str {
        match self {
            GearKind::Wetsuit => "Wetsuit",
            GearKind::Bottle => "Bottle",
            GearKind::WeightBelt => "Weights",
            GearKind::Bcd => "BCD",
            GearKind::Regulator => "Regulator",
            GearKind::DiveComputer => "Dive computer",
            GearKind::Mask => "Mask",
            GearKind::Fins => "Fins",
            GearKind::Torch => "Torch",
            GearKind::Camera => "Camera",
            GearKind::Other => "Other",
        }
    }
}

/// A piece of equipment tracked globally.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct GearItem {
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub id: u32,
    #[serde(default)]
    pub kind: GearKind,
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub notes: String,
}

/// How often and how recently a catalog entry was used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Usage {
    /// Number of dives that used the entry.
    pub dives: u32,
    /// Start time of the most recent of those dives.
    pub last_used: Option<Timestamp>,
}

impl Usage {
    fn record(&mut self, when: Timestamp) {
        self.dives += 1;
        self.last_used = Some(self.last_used.map_or(when, |last| last.max(when)));
    }
}

fn normalize(name: &str) -> String {
    name.trim().to_lowercase()
}

fn is_zero_u32(value: &u32) -> bool {
    *value == 0
}

impl DiveLog {
    /// Allocate a free person id.
    pub fn next_person_id(&self) -> u32 {
        self.people.iter().map(|p| p.id).max().unwrap_or(0) + 1
    }

    /// Allocate a free gear id.
    pub fn next_gear_id(&self) -> u32 {
        self.gear.iter().map(|g| g.id).max().unwrap_or(0) + 1
    }

    pub fn person_by_id(&self, id: u32) -> Option<&Person> {
        self.people.iter().find(|p| p.id == id)
    }

    pub fn gear_by_id(&self, id: u32) -> Option<&GearItem> {
        self.gear.iter().find(|g| g.id == id)
    }

    /// Case-insensitive lookup by name.
    pub fn person_by_name(&self, name: &str) -> Option<&Person> {
        let key = normalize(name);
        self.people
            .iter()
            .find(|person| normalize(&person.name) == key)
    }

    /// Case-insensitive lookup by kind and name.
    pub fn gear_by_name(&self, kind: GearKind, name: &str) -> Option<&GearItem> {
        let key = normalize(name);
        self.gear
            .iter()
            .find(|gear| gear.kind == kind && normalize(&gear.name) == key)
    }

    /// Usage of every registered person, keyed by id.
    pub fn person_usage(&self) -> BTreeMap<u32, Usage> {
        let mut usage: BTreeMap<u32, Usage> = self
            .people
            .iter()
            .map(|person| (person.id, Usage::default()))
            .collect();
        if self.people.is_empty() {
            return usage;
        }
        let index: HashMap<String, u32> = self
            .people
            .iter()
            .map(|person| (normalize(&person.name), person.id))
            .collect();
        for dive in &self.dives {
            let ids: BTreeSet<u32> = dive
                .people_names()
                .filter_map(|name| index.get(&normalize(name)).copied())
                .collect();
            for id in ids {
                if let Some(entry) = usage.get_mut(&id) {
                    entry.record(dive.when);
                }
            }
        }
        usage
    }

    /// Usage of every registered gear item, keyed by id.
    ///
    /// Suits, bottles and weights are matched by name; every other kind is
    /// used when a dive lists it in [`Dive::gear_ids`].
    pub fn gear_usage(&self) -> BTreeMap<u32, Usage> {
        let mut usage: BTreeMap<u32, Usage> = self
            .gear
            .iter()
            .map(|gear| (gear.id, Usage::default()))
            .collect();
        if self.gear.is_empty() {
            return usage;
        }

        let mut by_kind: HashMap<GearKind, HashMap<String, u32>> = HashMap::new();
        for gear in &self.gear {
            by_kind
                .entry(gear.kind)
                .or_default()
                .insert(normalize(&gear.name), gear.id);
        }
        let lookup = |kind: GearKind, name: &str| -> Option<u32> {
            by_kind.get(&kind)?.get(&normalize(name)).copied()
        };

        for dive in &self.dives {
            let mut ids: BTreeSet<u32> = dive.gear_ids.iter().copied().collect();
            if let Some(id) = lookup(GearKind::Wetsuit, &dive.suit) {
                ids.insert(id);
            }
            for cylinder in &dive.cylinders {
                if let Some(id) = lookup(GearKind::Bottle, &cylinder.description) {
                    ids.insert(id);
                }
            }
            for weights in &dive.weights {
                if let Some(id) = lookup(GearKind::WeightBelt, &weights.description) {
                    ids.insert(id);
                }
            }
            for computer in &dive.computers {
                if let Some(id) = lookup(GearKind::DiveComputer, &computer.model) {
                    ids.insert(id);
                }
            }
            for id in ids {
                if let Some(entry) = usage.get_mut(&id) {
                    entry.record(dive.when);
                }
            }
        }
        usage
    }

    /// Seed the catalog from dive history once per [`CATALOG_VERSION`], so
    /// names from older logs or Subsurface XML show up in the global lists
    /// without resurrecting entries the user deleted afterwards.
    pub fn seed_catalog(&mut self) {
        if self.catalog_version < 1 {
            self.adopt_untracked_entries();
        }
        if self.catalog_version < 2 {
            self.adopt_site_locations();
        }
        self.catalog_version = CATALOG_VERSION;
    }

    /// Register every person and gear name found in the dives that is not yet
    /// in the catalog. Returns how many entries were added.
    ///
    /// This is the migration path for logs imported before the catalog
    /// existed, and for files (Subsurface XML) that have no catalog of their
    /// own. It is deliberately not run on every load, so entries the user
    /// deleted do not come back.
    pub fn adopt_untracked_entries(&mut self) -> usize {
        let mut people: Vec<(String, String)> = Vec::new();
        let mut gear: Vec<(GearKind, String)> = Vec::new();
        for dive in &self.dives {
            people.extend(
                dive.people_names()
                    .map(|name| (normalize(name), name.to_string())),
            );
            if !dive.suit.trim().is_empty() {
                gear.push((GearKind::Wetsuit, dive.suit.trim().to_string()));
            }
            for cylinder in &dive.cylinders {
                if !cylinder.description.trim().is_empty() {
                    gear.push((GearKind::Bottle, cylinder.description.trim().to_string()));
                }
            }
            for weights in &dive.weights {
                if !weights.description.trim().is_empty() {
                    gear.push((GearKind::WeightBelt, weights.description.trim().to_string()));
                }
            }
            for computer in &dive.computers {
                let model = computer.model.trim();
                if !model.is_empty() && model != crate::model::MANUAL_COMPUTER_MODEL {
                    gear.push((GearKind::DiveComputer, model.to_string()));
                }
            }
        }

        let mut seen_people: HashSet<String> = self
            .people
            .iter()
            .map(|person| normalize(&person.name))
            .collect();
        let mut added = 0;
        for (key, name) in people {
            if key.is_empty() || !seen_people.insert(key) {
                continue;
            }
            let id = self.next_person_id();
            self.people.push(Person {
                id,
                name,
                notes: String::new(),
            });
            added += 1;
        }

        let mut seen_gear: HashSet<(GearKind, String)> = self
            .gear
            .iter()
            .map(|gear| (gear.kind, normalize(&gear.name)))
            .collect();
        for (kind, name) in gear {
            let key = normalize(&name);
            if key.is_empty() || !seen_gear.insert((kind, key)) {
                continue;
            }
            let id = self.next_gear_id();
            self.gear.push(GearItem {
                id,
                kind,
                name,
                notes: String::new(),
            });
            added += 1;
        }
        added
    }

    /// Merge the catalog of `other` into this log by name, returning maps from
    /// old ids to ids in this log. Used by [`DiveLog::merge`].
    pub(crate) fn merge_catalog(
        &mut self,
        other: &DiveLog,
    ) -> (HashMap<u32, u32>, HashMap<u32, u32>) {
        let mut people = HashMap::new();
        for person in &other.people {
            if let Some(existing) = self.person_by_name(&person.name) {
                people.insert(person.id, existing.id);
            } else {
                let id = self.next_person_id();
                people.insert(person.id, id);
                self.people.push(Person {
                    id,
                    name: person.name.clone(),
                    notes: person.notes.clone(),
                });
            }
        }

        let mut gear = HashMap::new();
        for item in &other.gear {
            if let Some(existing) = self.gear_by_name(item.kind, &item.name) {
                gear.insert(item.id, existing.id);
            } else {
                let id = self.next_gear_id();
                gear.insert(item.id, id);
                self.gear.push(GearItem {
                    id,
                    kind: item.kind,
                    name: item.name.clone(),
                    notes: item.notes.clone(),
                });
            }
        }
        (people, gear)
    }
}

/// Register the names and gear of one saved dive in the catalog. Returns the
/// `Command`s to add, ready to be dispatched before the dive update.
pub fn register_dive_entries(log: &DiveLog, dive: &Dive) -> Vec<crate::history::Command> {
    let mut commands = Vec::new();

    let mut known_people: HashSet<String> = log
        .people
        .iter()
        .map(|person| normalize(&person.name))
        .collect();
    let mut next_person = log.next_person_id();
    for name in dive.people_names() {
        let key = normalize(name);
        if key.is_empty() || !known_people.insert(key) {
            continue;
        }
        commands.push(crate::history::Command::AddPerson {
            person: Person {
                id: next_person,
                name: name.to_string(),
                notes: String::new(),
            },
        });
        next_person += 1;
    }

    let mut known_gear: HashSet<(GearKind, String)> = log
        .gear
        .iter()
        .map(|gear| (gear.kind, normalize(&gear.name)))
        .collect();
    let mut next_gear = log.next_gear_id();
    let mut add_gear = |kind: GearKind, name: &str, commands: &mut Vec<_>| {
        let key = normalize(name);
        if key.is_empty() || !known_gear.insert((kind, key)) {
            return;
        }
        commands.push(crate::history::Command::AddGear {
            gear: GearItem {
                id: next_gear,
                kind,
                name: name.to_string(),
                notes: String::new(),
            },
        });
        next_gear += 1;
    };
    add_gear(GearKind::Wetsuit, &dive.suit, &mut commands);
    for cylinder in &dive.cylinders {
        add_gear(GearKind::Bottle, &cylinder.description, &mut commands);
    }
    for weights in &dive.weights {
        add_gear(GearKind::WeightBelt, &weights.description, &mut commands);
    }
    for computer in &dive.computers {
        let model = computer.model.trim();
        if !model.is_empty() && model != crate::model::MANUAL_COMPUTER_MODEL {
            add_gear(GearKind::DiveComputer, model, &mut commands);
        }
    }

    commands
}

/// Replace one name in a comma-separated list, ignoring case. Returns `None`
/// when nothing changed.
fn rename_in_list(list: &str, old: &str, new: &str) -> Option<String> {
    let mut changed = false;
    let renamed: Vec<&str> = list
        .split(',')
        .map(|part| {
            let name = part.trim();
            if name.eq_ignore_ascii_case(old) {
                changed = true;
                new
            } else {
                name
            }
        })
        .filter(|name| !name.is_empty())
        .collect();
    changed.then(|| renamed.join(", "))
}

/// Build the commands that rename a person, updating the dives that mention
/// them so usage keeps pointing at the same entry.
pub fn rename_person_commands(
    log: &DiveLog,
    id: u32,
    new_name: &str,
) -> Vec<crate::history::Command> {
    let new_name = new_name.trim();
    let Some(before) = log.person_by_id(id).cloned() else {
        return Vec::new();
    };
    if new_name.is_empty()
        || before.name == new_name
        || log
            .person_by_name(new_name)
            .is_some_and(|person| person.id != id)
    {
        return Vec::new();
    }
    let mut commands = vec![crate::history::Command::UpdatePerson {
        before: before.clone(),
        after: Person {
            name: new_name.to_string(),
            ..before.clone()
        },
    }];
    for dive in &log.dives {
        let buddy = rename_in_list(&dive.buddy, &before.name, new_name);
        let diveguide = rename_in_list(&dive.diveguide, &before.name, new_name);
        if buddy.is_some() || diveguide.is_some() {
            let mut after = dive.clone();
            if let Some(buddy) = buddy {
                after.buddy = buddy;
            }
            if let Some(diveguide) = diveguide {
                after.diveguide = diveguide;
            }
            commands.push(crate::history::Command::UpdateDive {
                before: dive.clone(),
                after,
            });
        }
    }
    commands
}

/// Build the commands that rename a gear item, updating the dive fields it is
/// linked to (suit, cylinder or weight descriptions) so usage is preserved.
pub fn rename_gear_commands(
    log: &DiveLog,
    id: u32,
    new_name: &str,
) -> Vec<crate::history::Command> {
    let new_name = new_name.trim();
    let Some(before) = log.gear_by_id(id).cloned() else {
        return Vec::new();
    };
    if new_name.is_empty()
        || before.name == new_name
        || log
            .gear_by_name(before.kind, new_name)
            .is_some_and(|gear| gear.id != id)
    {
        return Vec::new();
    }
    let mut commands = vec![crate::history::Command::UpdateGear {
        before: before.clone(),
        after: GearItem {
            name: new_name.to_string(),
            ..before.clone()
        },
    }];

    let matches = |value: &str| value.trim().eq_ignore_ascii_case(&before.name);
    for dive in &log.dives {
        let mut after = dive.clone();
        match before.kind {
            GearKind::Wetsuit if matches(&dive.suit) => after.suit = new_name.to_string(),
            GearKind::Bottle => {
                for cylinder in &mut after.cylinders {
                    if matches(&cylinder.description) {
                        cylinder.description = new_name.to_string();
                    }
                }
            }
            GearKind::WeightBelt => {
                for weights in &mut after.weights {
                    if matches(&weights.description) {
                        weights.description = new_name.to_string();
                    }
                }
            }
            GearKind::DiveComputer => {
                for computer in &mut after.computers {
                    if matches(&computer.model) {
                        computer.model = new_name.to_string();
                    }
                }
            }
            _ => {}
        }
        if after != *dive {
            commands.push(crate::history::Command::UpdateDive {
                before: dive.clone(),
                after,
            });
        }
    }
    commands
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Cylinder, Dive, WeightSystem};
    use crate::units::{Timestamp, Weight};

    fn dive(id: u32, when: Timestamp) -> Dive {
        Dive {
            id,
            when,
            ..Default::default()
        }
    }

    #[test]
    fn adoption_registers_people_and_gear_from_dives() {
        let mut log = DiveLog::new();
        log.dives.push(Dive {
            buddy: "Alice, Bob".into(),
            diveguide: "Carol".into(),
            suit: "7mm wetsuit".into(),
            cylinders: vec![Cylinder {
                description: "D12".into(),
                ..Default::default()
            }],
            weights: vec![WeightSystem::new(Weight::from_kg(4.0), "belt")],
            computers: vec![crate::model::DiveComputer {
                model: "Perdix".into(),
                ..Default::default()
            }],
            ..Default::default()
        });

        assert_eq!(log.adopt_untracked_entries(), 7);
        assert_eq!(log.people.len(), 3);
        assert_eq!(log.gear.len(), 4);
        assert!(log.person_by_name("alice").is_some());
        assert!(log.gear_by_name(GearKind::Bottle, "d12").is_some());
        assert!(log.gear_by_name(GearKind::DiveComputer, "perdix").is_some());

        // Idempotent.
        assert_eq!(log.adopt_untracked_entries(), 0);
    }

    #[test]
    fn usage_orders_by_most_recent_dive() {
        let mut log = DiveLog::new();
        log.people.push(Person {
            id: 1,
            name: "Alice".into(),
            ..Default::default()
        });
        log.people.push(Person {
            id: 2,
            name: "Bob".into(),
            ..Default::default()
        });
        log.gear.push(GearItem {
            id: 1,
            kind: GearKind::Wetsuit,
            name: "7mm".into(),
            ..Default::default()
        });
        log.gear.push(GearItem {
            id: 2,
            kind: GearKind::Regulator,
            name: "MK25".into(),
            ..Default::default()
        });
        log.gear.push(GearItem {
            id: 3,
            kind: GearKind::DiveComputer,
            name: "Perdix".into(),
            ..Default::default()
        });

        let mut first = dive(1, 1_000);
        first.buddy = "ALICE".into();
        first.suit = "7mm".into();
        let mut second = dive(2, 2_000);
        second.diveguide = "alice".into();
        second.gear_ids = vec![2];
        second.computers = vec![crate::model::DiveComputer {
            model: "perdix".into(),
            ..Default::default()
        }];
        log.dives.extend([first, second]);

        let people = log.person_usage();
        assert_eq!(people[&1].dives, 2);
        assert_eq!(people[&1].last_used, Some(2_000));
        assert_eq!(people[&2], Usage::default());

        let gear = log.gear_usage();
        assert_eq!(gear[&1].dives, 1);
        assert_eq!(gear[&2].dives, 1);
        assert_eq!(gear[&2].last_used, Some(2_000));
        assert_eq!(gear[&3].dives, 1);
        assert_eq!(gear[&3].last_used, Some(2_000));
    }

    #[test]
    fn merge_deduplicates_and_remaps_gear_ids() {
        let mut log = DiveLog::new();
        log.people.push(Person {
            id: 4,
            name: "Alice".into(),
            ..Default::default()
        });
        log.gear.push(GearItem {
            id: 7,
            kind: GearKind::Bcd,
            name: "Wing".into(),
            ..Default::default()
        });

        let mut other = DiveLog::new();
        other.people.push(Person {
            id: 1,
            name: "Alice".into(),
            ..Default::default()
        });
        other.people.push(Person {
            id: 2,
            name: "Bob".into(),
            ..Default::default()
        });
        other.gear.push(GearItem {
            id: 3,
            kind: GearKind::Bcd,
            name: "wing".into(),
            ..Default::default()
        });
        other.gear.push(GearItem {
            id: 5,
            kind: GearKind::Bcd,
            name: "Other".into(),
            ..Default::default()
        });
        let mut merged = dive(1, 5_000);
        merged.gear_ids = vec![3, 5];
        other.dives.push(merged);

        log.merge(other);
        assert_eq!(log.people.len(), 2);
        assert_eq!(log.gear.len(), 2);
        let dive = &log.dives[0];
        let other_bcd = log.gear_by_name(GearKind::Bcd, "Other").unwrap().id;
        assert_eq!(dive.gear_ids, vec![7, other_bcd]);
    }

    #[test]
    fn renaming_updates_dive_fields_and_keeps_usage() {
        let mut log = DiveLog::new();
        log.people.push(Person {
            id: 1,
            name: "Camille".into(),
            ..Default::default()
        });
        log.gear.push(GearItem {
            id: 1,
            kind: GearKind::Wetsuit,
            name: "7mm".into(),
            ..Default::default()
        });
        let mut d = dive(1, 1_000);
        d.buddy = "camille, Sam".into();
        d.suit = "7mm".into();
        log.dives.push(d);

        let commands = rename_person_commands(&log, 1, "Camille D.");
        for command in &commands {
            command.apply(&mut log);
        }
        let commands = rename_gear_commands(&log, 1, "7 mm wetsuit");
        for command in &commands {
            command.apply(&mut log);
        }

        assert_eq!(log.dives[0].buddy, "Camille D., Sam");
        assert_eq!(log.dives[0].suit, "7 mm wetsuit");
        assert_eq!(log.person_usage()[&1].dives, 1);
        assert_eq!(log.gear_usage()[&1].dives, 1);
    }

    #[test]
    fn register_dive_entries_adds_only_missing_names() {
        let mut log = DiveLog::new();
        log.people.push(Person {
            id: 1,
            name: "Alice".into(),
            ..Default::default()
        });
        let mut saved = dive(1, 1_000);
        saved.buddy = "alice, Bob".into();
        saved.suit = "Dry suit".into();

        let commands = register_dive_entries(&log, &saved);
        assert_eq!(commands.len(), 2);
        for command in &commands {
            command.apply(&mut log);
        }
        assert_eq!(log.people.len(), 2);
        assert_eq!(log.people[1].id, 2);
        assert_eq!(log.gear.len(), 1);
        assert_eq!(log.gear[0].id, 1);
    }
}
