//! High-level user actions.
//!
//! Each action builds one or more [`Command`]s and dispatches them through the
//! [`AppState`] so that it is automatically undoable.

use benthic_core::{Command, Dive, DivePlan, DiveTrip, GearItem, GearKind, Person};
use dioxus::prelude::WritableExt;

use crate::i18n;
use crate::state::{AppState, ImportMode, PendingImport};

/// The example logbook shipped inside the binary, so the app can offer sample
/// data without a file dialog.
pub const EXAMPLE_LOG: &str = include_str!("../dives/demo.ssrf");

/// Days between consecutive dives before automatic grouping breaks a trip.
pub const AUTOGROUP_MAX_GAP_DAYS: i64 = 3;

/// Maximum distance between two same-named sites for them to be considered
/// duplicates.
pub const SITE_DEDUP_RADIUS_M: f64 = 500.0;

/// Seed the people/gear catalog from dive history.
///
/// Runs once per log (and for freshly imported files), so names from older
/// versions or Subsurface XML show up in the global lists without resurrecting
/// entries the user deleted afterwards.
pub fn seed_catalog(log: &mut benthic_core::DiveLog) {
    if log.catalog_version >= benthic_core::CATALOG_VERSION {
        return;
    }
    log.adopt_untracked_entries();
    log.catalog_version = benthic_core::CATALOG_VERSION;
}

/// Create an empty, manually-entered dive, select it, and open the editor.
pub fn new_dive(state: AppState) {
    let log = (state.log)();
    let mut dive = Dive::manual(crate::platform::now_secs());
    dive.id = log.next_id();
    dive.number = log.dives.iter().map(|d| d.number).max().unwrap_or(0) + 1;
    dive.salinity = Some((state.prefs)().default_salinity.value());
    let index = log.dives.len();
    state.dispatch(Command::AddDive {
        dive: dive.clone(),
        index,
    });
    let mut selected = state.selected;
    selected.set(Some(dive.id));
    let mut show_trip = state.show_trip;
    show_trip.set(None);
    state.set_status(state.strings().added_dive);
}

/// Duplicate a dive, assigning it a fresh id and number.
pub fn duplicate_dive(state: AppState, id: u32) {
    let log = (state.log)();
    let Some(original) = log.dive_by_id(id).cloned() else {
        return;
    };
    let mut copy = original;
    copy.id = log.next_id();
    copy.number = log.dives.iter().map(|d| d.number).max().unwrap_or(0) + 1;
    copy.computers = copy
        .computers
        .iter()
        .map(|dc| {
            let mut dc = dc.clone();
            dc.dive_id = 0;
            dc
        })
        .collect();
    let index = log.dives.len();
    state.dispatch(Command::AddDive {
        dive: copy.clone(),
        index,
    });
    let mut selected = state.selected;
    selected.set(Some(copy.id));
    let mut show_trip = state.show_trip;
    show_trip.set(None);
    state.set_status(state.strings().duplicated_dive);
}

/// Delete a dive and select a neighbour.
pub fn delete_dive(state: AppState, id: u32) {
    let log = (state.log)();
    let Some((index, dive)) = log
        .dives
        .iter()
        .enumerate()
        .find(|(_, d)| d.id == id)
        .map(|(i, d)| (i, d.clone()))
    else {
        return;
    };
    let neighbor = log
        .dives_sorted()
        .into_iter()
        .filter(|d| d.id != id)
        .min_by_key(|d| (d.when - dive.when).unsigned_abs())
        .map(|d| d.id);
    state.dispatch(Command::DeleteDive { dive, index });
    let mut selected = state.selected;
    selected.set(neighbor);
    state.set_status(state.strings().deleted_dive);
}

/// Delete every ticked dive as a single undo step.
pub fn delete_selected(state: AppState) {
    let ids: Vec<u32> = (state.selection)().iter().copied().collect();
    if ids.is_empty() {
        return;
    }
    let log = (state.log)();
    let commands = benthic_core::history::delete_dives(&log, &ids);
    state.dispatch_all(
        i18n::t1(state.strings().undo_delete_dives, ids.len()),
        commands,
    );

    let mut selection = state.selection;
    selection.write().clear();

    let mut selected = state.selected;
    if selected().is_some_and(|id| ids.contains(&id)) {
        selected.set((state.log)().dives_recent_first().first().map(|d| d.id));
    }
    state.set_status(i18n::t1(state.strings().deleted_dives, ids.len()));
}

/// Fields a batch edit may change. `None` leaves a field untouched.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BatchEdit {
    /// `Some(None)` removes the trip assignment, `Some(Some(id))` assigns it.
    pub trip_id: Option<Option<u32>>,
    pub buddy: Option<String>,
    pub diveguide: Option<String>,
    pub suit: Option<String>,
    /// `Some(0)` clears the rating.
    pub rating: Option<u8>,
    pub add_tags: Vec<String>,
    pub remove_tags: Vec<String>,
}

impl BatchEdit {
    pub fn is_empty(&self) -> bool {
        self.trip_id.is_none()
            && self.buddy.is_none()
            && self.diveguide.is_none()
            && self.suit.is_none()
            && self.rating.is_none()
            && self.add_tags.is_empty()
            && self.remove_tags.is_empty()
    }
}

/// Open the batch-edit dialog for the ticked dives.
pub fn open_batch_edit(state: AppState) {
    if (state.selection)().is_empty() {
        return;
    }
    let mut show = state.show_batch_edit;
    show.set(true);
}

/// Apply a batch edit to every ticked dive as one undo step.
pub fn apply_batch_edit(state: AppState, edit: BatchEdit) {
    let ids: std::collections::BTreeSet<u32> = (state.selection)().iter().copied().collect();
    if ids.is_empty() || edit.is_empty() {
        let mut show = state.show_batch_edit;
        show.set(false);
        return;
    }
    let log = (state.log)();
    let mut commands = Vec::new();
    for dive in log.dives.iter().filter(|dive| ids.contains(&dive.id)) {
        let mut after = dive.clone();
        if let Some(trip_id) = edit.trip_id {
            after.trip_id = trip_id;
        }
        if let Some(buddy) = &edit.buddy {
            after.buddy.clone_from(buddy);
        }
        if let Some(diveguide) = &edit.diveguide {
            after.diveguide.clone_from(diveguide);
        }
        if let Some(suit) = &edit.suit {
            after.suit.clone_from(suit);
        }
        if let Some(rating) = edit.rating {
            after.rating = rating;
        }
        for tag in &edit.add_tags {
            if !after.tags.iter().any(|t| t.eq_ignore_ascii_case(tag)) {
                after.tags.push(tag.clone());
            }
        }
        after
            .tags
            .retain(|tag| !edit.remove_tags.iter().any(|r| r.eq_ignore_ascii_case(tag)));
        after.tags.sort();
        after.tags.dedup();
        if after != *dive {
            commands.push(Command::UpdateDive {
                before: dive.clone(),
                after,
            });
        }
    }
    if commands.is_empty() {
        let mut show = state.show_batch_edit;
        show.set(false);
        return;
    }
    let count = commands.len();
    state.dispatch_all(i18n::t1(state.strings().undo_batch_edit, count), commands);
    let mut selection = state.selection;
    selection.write().clear();
    let mut show = state.show_batch_edit;
    show.set(false);
    state.set_status(i18n::t1(state.strings().edited_dives, count));
}

/// Create a trip from the ticked dives and assign them to it.
pub fn create_trip_from_selection(state: AppState) {
    let _ = create_trip_with_selection(state, "");
}

/// Create a trip, optionally using the current selection. An empty name falls
/// back to the first selected dive's site, or a generated trip label.
pub fn create_trip_with_selection(state: AppState, name: &str) -> Option<u32> {
    let log = (state.log)();
    let ids: std::collections::BTreeSet<u32> = (state.selection)().iter().copied().collect();
    let selected: Vec<&Dive> = log.dives.iter().filter(|d| ids.contains(&d.id)).collect();

    let id = log.next_trip_id();
    let date = selected.iter().map(|d| d.when).min();
    let location = if !name.trim().is_empty() {
        name.trim().to_string()
    } else {
        selected
            .first()
            .and_then(|d| log.site_name_of(d))
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| i18n::t1(state.strings().trip_fallback, id))
    };

    let mut commands = vec![Command::AddTrip {
        trip: DiveTrip {
            id,
            date,
            location,
            ..Default::default()
        },
    }];
    for dive in selected {
        let mut after = dive.clone();
        after.trip_id = Some(id);
        commands.push(Command::UpdateDive {
            before: dive.clone(),
            after,
        });
    }

    let count = commands.len() - 1;
    let mut selection = state.selection;
    selection.write().clear();
    state.dispatch_all(state.strings().undo_create_trip, commands);
    state.set_status(i18n::t1(state.strings().created_trip, count));
    Some(id)
}

/// Move the selected dives into `trip_id`; `None` detaches them.
pub fn move_selection_to_trip(mut state: AppState, trip_id: Option<u32>) {
    let ids: std::collections::BTreeSet<u32> = (state.selection)().iter().copied().collect();
    assign_dives_to_trip(state, &ids, trip_id);
    state.selection.write().clear();
}

/// Detach one dive from its trip.
pub fn detach_dive(state: AppState, dive_id: u32) {
    assign_dives_to_trip(state, &std::collections::BTreeSet::from([dive_id]), None);
}

fn assign_dives_to_trip(
    state: AppState,
    ids: &std::collections::BTreeSet<u32>,
    trip_id: Option<u32>,
) {
    if ids.is_empty() {
        return;
    }
    let log = (state.log)();
    let mut commands = Vec::new();
    for dive in log.dives.iter().filter(|d| ids.contains(&d.id)) {
        if dive.trip_id == trip_id {
            continue;
        }
        let mut after = dive.clone();
        after.trip_id = trip_id;
        commands.push(Command::UpdateDive {
            before: dive.clone(),
            after,
        });
    }
    if commands.is_empty() {
        return;
    }
    let count = commands.len();
    state.dispatch_all(state.strings().undo_move_dives, commands);
    let status = if trip_id.is_some() {
        i18n::t1(state.strings().moved_dives, count)
    } else {
        i18n::t1(state.strings().detached_dives, count)
    };
    state.set_status(status);
}

/// Rename a trip (no-op when unchanged).
pub fn rename_trip(state: AppState, id: u32, name: &str) {
    let log = (state.log)();
    let Some(before) = log.trip_by_id(id).cloned() else {
        return;
    };
    let mut after = before.clone();
    after.location = name.trim().to_string();
    if after == before {
        return;
    }
    state.dispatch(Command::UpdateTrip { before, after });
    state.set_status(state.strings().renamed_trip);
}

/// Delete a trip, unassigning its dives as one undo step.
pub fn delete_trip(state: AppState, id: u32) {
    let log = (state.log)();
    let commands = benthic_core::history::delete_trip(&log, id);
    if commands.is_empty() {
        return;
    }
    state.dispatch_all(state.strings().undo_delete_trip, commands);
    // If the deleted trip was open in the detail pane, fall back to the dive
    // view so the pane is never left blank.
    if (state.show_trip)() == Some(id) {
        let mut show_trip = state.show_trip;
        show_trip.set(None);
    }
    state.set_status(state.strings().deleted_trip);
}

/// Merge dive sites that share a name (and are close, or lack coordinates).
pub fn merge_duplicate_sites(state: AppState) {
    let before = (state.log)();
    let mut after = before.clone();
    let removed = after.merge_duplicate_sites(SITE_DEDUP_RADIUS_M);
    if removed == 0 {
        state.set_status(state.strings().no_duplicate_sites);
        return;
    }
    state.dispatch(Command::Snapshot {
        label: state.strings().undo_merge_sites.into(),
        before: Box::new(before),
        after: Box::new(after),
    });
    state.set_status(i18n::t1(state.strings().merged_sites, removed));
}

/// Restore the most recent automatic backup over the current log.
pub fn restore_backup(state: AppState) {
    let Some(text) = crate::storage::load_backup() else {
        state.set_status(state.strings().no_backup);
        return;
    };
    match benthic_core::io::parse_auto(&text) {
        Ok(mut parsed) => {
            parsed.number_missing_dives();
            let before = (state.log)();
            state.dispatch(Command::Snapshot {
                label: state.strings().undo_restore_backup.into(),
                before: Box::new(before),
                after: Box::new(parsed),
            });
            state.set_status(state.strings().restored_backup);
        }
        Err(e) => state.set_status(i18n::t1(state.strings().backup_unreadable, e)),
    }
}

/// Export the log as Subsurface XML (browser download or file write).
pub fn export_ssrf(state: AppState) {
    let log = (state.log)();
    let text = benthic_core::io::ssrf::write_string(&log);
    match crate::platform::save_file("benthic.ssrf", &text) {
        Ok(message) => state.set_status(message),
        Err(e) => state.set_status(i18n::t1(state.strings().export_failed, e)),
    }
}

/// Open the preferences dialog.
pub fn open_preferences(state: AppState) {
    let mut show = state.show_prefs;
    show.set(true);
}

/// Open the trips manager dialog.
pub fn open_trips(state: AppState) {
    let mut show = state.show_trips;
    show.set(true);
}

/// Add a person to the global list. Empty and duplicate names are ignored.
pub fn add_person(state: AppState, name: &str) -> Option<u32> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let log = (state.log)();
    if log.person_by_name(name).is_some() {
        return None;
    }
    let id = log.next_person_id();
    state.dispatch(Command::AddPerson {
        person: Person {
            id,
            name: name.to_string(),
            notes: String::new(),
        },
    });
    Some(id)
}

/// Rename a person, updating the dives that mention them.
pub fn rename_person(state: AppState, id: u32, name: &str) {
    let log = (state.log)();
    let commands = benthic_core::rename_person_commands(&log, id, name);
    if commands.is_empty() {
        return;
    }
    state.dispatch_all("Edit person", commands);
}

pub fn delete_person(state: AppState, id: u32) {
    let log = (state.log)();
    let Some(person) = log.person_by_id(id).cloned() else {
        return;
    };
    state.dispatch(Command::DeletePerson { person });
}

/// Add a gear item to the global list. Empty and duplicate names are ignored.
pub fn add_gear(state: AppState, kind: GearKind, name: &str) -> Option<u32> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let log = (state.log)();
    if log.gear_by_name(kind, name).is_some() {
        return None;
    }
    let id = log.next_gear_id();
    state.dispatch(Command::AddGear {
        gear: GearItem {
            id,
            kind,
            name: name.to_string(),
            notes: String::new(),
        },
    });
    Some(id)
}

/// Rename a gear item, updating the dive fields it is linked to.
pub fn rename_gear(state: AppState, id: u32, name: &str) {
    let log = (state.log)();
    let commands = benthic_core::rename_gear_commands(&log, id, name);
    if commands.is_empty() {
        return;
    }
    state.dispatch_all("Edit gear", commands);
}

/// Change the kind of a gear item.
pub fn update_gear_kind(state: AppState, id: u32, kind: GearKind) {
    let log = (state.log)();
    let Some(before) = log.gear_by_id(id).cloned() else {
        return;
    };
    if before.kind == kind {
        return;
    }
    let after = GearItem {
        kind,
        ..before.clone()
    };
    state.dispatch(Command::UpdateGear { before, after });
}

/// Delete a gear item and unlink it from every dive, as one undo step.
pub fn delete_gear(state: AppState, id: u32) {
    let log = (state.log)();
    let commands = benthic_core::history::delete_gear(&log, id);
    if commands.is_empty() {
        return;
    }
    state.dispatch_all("Delete gear", commands);
}

/// Open the dive planner dialog.
pub fn open_planner(state: AppState) {
    let mut show = state.show_planner;
    show.set(true);
}

/// Open the two-dive comparison dialog.
pub fn open_compare(state: AppState) {
    let mut show = state.show_compare;
    show.set(true);
}

/// Save a computed plan as a new dive.
pub fn save_plan(state: AppState, plan: DivePlan) {
    let log = (state.log)();
    let salinity = (state.prefs)().default_salinity.value();
    let mut dive = plan.to_dive(crate::platform::now_secs(), salinity);
    dive.id = log.next_id();
    dive.number = log.dives.iter().map(|d| d.number).max().unwrap_or(0) + 1;
    let index = log.dives.len();
    state.dispatch(Command::AddDive {
        dive: dive.clone(),
        index,
    });
    let mut selected = state.selected;
    selected.set(Some(dive.id));
    let mut show_trip = state.show_trip;
    show_trip.set(None);
    state.set_status(state.strings().saved_plan);
}

/// Merge the `remove` trip into the `keep` trip as one undo step.
pub fn merge_trips(state: AppState, keep: u32, remove: u32) {
    let before = (state.log)();
    let mut after = before.clone();
    if !after.merge_trips(keep, remove) {
        return;
    }
    state.dispatch(Command::Snapshot {
        label: state.strings().undo_merge_trips.into(),
        before: Box::new(before),
        after: Box::new(after),
    });
    // Follow the merge if the removed trip was the one on screen.
    if (state.show_trip)() == Some(remove) {
        let mut show_trip = state.show_trip;
        show_trip.set(Some(keep));
    }
    state.set_status(state.strings().merged_trips);
}

/// Toggle automatic trip grouping on/off, regrouping or ungrouping as needed.
pub fn toggle_autogroup(state: AppState) {
    let mut after = (state.log)();
    let before = after.clone();
    let t = state.strings();
    let (label, message) = if after.autogroup {
        after.autogroup = false;
        after.clear_auto_trips();
        (t.autogroup_off, t.autogroup_disabled.to_string())
    } else {
        after.autogroup = true;
        let created = after.autogroup_trips(AUTOGROUP_MAX_GAP_DAYS);
        (t.autogroup_on, i18n::t1(t.autogroup_created, created))
    };
    state.dispatch(Command::Snapshot {
        label: label.into(),
        before: Box::new(before),
        after: Box::new(after),
    });
    state.set_status(message);
}

/// Whether a downloaded dive already exists in the log.
///
/// Exact start-time/model matches are the common case. Subsurface XML stores
/// wall-clock times while downloaded dives are UTC, so also accept the same
/// model/profile shifted by the recorded UTC offset. This keeps imported
/// Subsurface logs from being duplicated without treating unrelated dives as
/// the same entry.
#[cfg_attr(
    not(any(feature = "divecomputer", target_arch = "wasm32")),
    allow(dead_code)
)]
fn same_downloaded_dive(existing: &Dive, incoming: &Dive) -> bool {
    let model = |dive: &Dive| {
        dive.primary_computer()
            .map(|computer| computer.model.clone())
            .unwrap_or_default()
    };
    if model(existing) != model(incoming) {
        return false;
    }
    if existing.when == incoming.when {
        return true;
    }
    let Some(offset) = incoming
        .utc_offset_seconds()
        .or_else(|| existing.utc_offset_seconds())
    else {
        return false;
    };
    let delta = (existing.when - incoming.when).abs();
    if (delta - offset.unsigned_abs() as i64).abs() > 2 {
        return false;
    }
    existing.duration().is_some()
        && existing.duration() == incoming.duration()
        && existing.max_depth().is_some()
        && existing.max_depth() == incoming.max_depth()
}

/// Incoming dive ids that already exist in `existing`, using the same
/// time/model/profile matching as dive-computer downloads.
pub fn duplicate_dive_ids(
    existing: &benthic_core::DiveLog,
    incoming: &benthic_core::DiveLog,
) -> std::collections::BTreeSet<u32> {
    incoming
        .dives
        .iter()
        .filter(|dive| {
            existing
                .dives
                .iter()
                .any(|candidate| same_downloaded_dive(candidate, dive))
        })
        .map(|dive| dive.id)
        .collect()
}

/// Queue downloaded dives for per-dive review instead of adding them directly.
#[cfg_attr(
    not(any(feature = "divecomputer", target_arch = "wasm32")),
    allow(dead_code)
)]
pub fn review_downloaded(
    state: AppState,
    mut dives: Vec<Dive>,
    source: impl Into<String>,
    fingerprint: Option<(String, Vec<u8>)>,
) -> usize {
    // Downloaded dives arrive without log ids; the review dialog uses ids as
    // its selection keys, so give each one a distinct id first.
    for (index, dive) in dives.iter_mut().enumerate() {
        dive.id = index as u32 + 1;
    }
    let incoming = benthic_core::DiveLog {
        dives,
        ..Default::default()
    };
    let duplicates = duplicate_dive_ids(&(state.log)(), &incoming);
    let count = incoming.dives.len();
    let mut import_review = state.import_review;
    import_review.set(Some(PendingImport {
        log: incoming,
        file_name: source.into(),
        mode: ImportMode::Download,
        duplicates,
        fingerprint,
        nonce: crate::platform::now_secs() as u64,
    }));
    count
}

/// Replace the entire log with an imported one as a single undoable step.
///
/// This backs the "open" action: the current log (and the autosaved copy,
/// which is rewritten on the next autosave) is discarded in favour of the
/// file's contents. The previous log stays on the undo stack and in the
/// automatic backup, so an accidental open is recoverable.
pub fn open_log(state: AppState, mut incoming: benthic_core::DiveLog) -> (usize, usize) {
    incoming.fixup_all();
    incoming.number_missing_dives();
    seed_catalog(&mut incoming);
    let dives = incoming.dives.len();
    let sites = incoming.sites.len();
    let before = (state.log)();
    let mut filter = state.filter;
    let mut selection = state.selection;
    filter.set(benthic_core::DiveFilter::default());
    selection.write().clear();
    state.dispatch(Command::Snapshot {
        label: i18n::t2(state.strings().undo_open, dives, sites),
        before: Box::new(before),
        after: Box::new(incoming),
    });
    (dives, sites)
}

/// Replace the log with the bundled example logbook as one undoable step, and
/// select the most recent dive.
pub fn open_example_log(state: AppState) {
    match benthic_core::io::parse_auto(EXAMPLE_LOG) {
        Ok(parsed) => {
            let dives = parsed.dives.len();
            open_log(state, parsed);
            let mut selected = state.selected;
            selected.set((state.log)().dives_recent_first().first().map(|d| d.id));
            state.set_status(i18n::t1(state.strings().loaded_demo, dives));
        }
        Err(error) => state.set_status(i18n::t1(state.strings().demo_failed, error)),
    }
}

/// Merge an imported log into the current one as a single undoable step.
/// Returns the number of dives and sites added.
pub fn merge_log(state: AppState, mut incoming: benthic_core::DiveLog) -> (usize, usize) {
    incoming.fixup_all();
    seed_catalog(&mut incoming);
    let dives = incoming.dives.len();
    let sites = incoming.sites.len();
    let before = (state.log)();
    let mut after = before.clone();
    after.merge(incoming);
    after.number_missing_dives();
    seed_catalog(&mut after);
    state.dispatch(Command::Snapshot {
        label: i18n::t2(state.strings().undo_import, dives, sites),
        before: Box::new(before),
        after: Box::new(after),
    });
    (dives, sites)
}

#[cfg(test)]
mod tests {
    use super::*;
    use benthic_core::units::{Depth, Duration};
    use benthic_core::DiveComputer;

    fn dive(when: i64, offset: Option<i32>) -> Dive {
        Dive {
            when,
            duration: Some(Duration::from_minutes(30)),
            max_depth: Some(Depth::from_meters(20.0)),
            computers: vec![DiveComputer {
                model: "Seac Screen".to_string(),
                timezone_offset: offset,
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn downloaded_dives_match_subsurface_wall_clock_times() {
        let imported = dive(1_000_000, Some(3600));
        let downloaded = dive(1_000_000 - 3600, Some(3600));
        assert!(same_downloaded_dive(&imported, &downloaded));
    }

    #[test]
    fn different_profiles_are_not_duplicates() {
        let imported = dive(1_000_000, Some(3600));
        let mut downloaded = dive(1_000_000 - 3600, Some(3600));
        downloaded.max_depth = Some(Depth::from_meters(25.0));
        assert!(!same_downloaded_dive(&imported, &downloaded));
    }
}
