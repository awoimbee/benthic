//! High-level user actions.
//!
//! Each action builds one or more [`Command`]s and dispatches them through the
//! [`AppState`] so that it is automatically undoable.

use benthic_core::{Command, Dive, DivePlan, DiveTrip};
use dioxus::prelude::WritableExt;

use crate::i18n;
use crate::state::AppState;

/// Days between consecutive dives before automatic grouping breaks a trip.
pub const AUTOGROUP_MAX_GAP_DAYS: i64 = 3;

/// Maximum distance between two same-named sites for them to be considered
/// duplicates.
pub const SITE_DEDUP_RADIUS_M: f64 = 500.0;

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

/// Create a trip from the ticked dives and assign them to it.
pub fn create_trip_from_selection(state: AppState) {
    let ids: Vec<u32> = (state.selection)().iter().copied().collect();
    if ids.is_empty() {
        return;
    }
    let log = (state.log)();
    let selected: Vec<&Dive> = log.dives.iter().filter(|d| ids.contains(&d.id)).collect();
    if selected.is_empty() {
        return;
    }

    let id = log.next_trip_id();
    let date = selected.iter().map(|d| d.when).min();
    let location = selected
        .first()
        .and_then(|d| log.site_name_of(d))
        .unwrap_or("")
        .to_string();

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
    state.dispatch_all(state.strings().undo_create_trip, commands);
    let mut selection = state.selection;
    selection.write().clear();
    state.set_status(i18n::t1(state.strings().created_trip, count));
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
        Ok(parsed) => {
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

/// Merge dives downloaded from a dive computer, skipping any already present
/// (matched by start time and computer model). Returns `(added, skipped)`.
#[cfg(any(feature = "divecomputer", target_arch = "wasm32"))]
pub fn merge_downloaded(state: AppState, dives: Vec<Dive>) -> (usize, usize) {
    let log = (state.log)();
    let key_of = |dive: &Dive| {
        (
            dive.when,
            dive.primary_computer()
                .map(|c| c.model.clone())
                .unwrap_or_default(),
        )
    };
    let existing: std::collections::BTreeSet<(i64, String)> =
        log.dives.iter().map(key_of).collect();
    let mut fresh = Vec::new();
    let mut skipped = 0;
    for dive in dives {
        if existing.contains(&key_of(&dive)) {
            skipped += 1;
        } else {
            fresh.push(dive);
        }
    }
    let added = fresh.len();
    if added > 0 {
        let incoming = benthic_core::DiveLog {
            dives: fresh,
            ..Default::default()
        };
        merge_log(state, incoming);
    }
    (added, skipped)
}

/// Replace the entire log with an imported one as a single undoable step.
///
/// This backs the "open" action: the current log (and the autosaved copy,
/// which is rewritten on the next autosave) is discarded in favour of the
/// file's contents. The previous log stays on the undo stack and in the
/// automatic backup, so an accidental open is recoverable.
pub fn open_log(state: AppState, mut incoming: benthic_core::DiveLog) -> (usize, usize) {
    incoming.fixup_all();
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

/// Merge an imported log into the current one as a single undoable step.
/// Returns the number of dives and sites added.
pub fn merge_log(state: AppState, mut incoming: benthic_core::DiveLog) -> (usize, usize) {
    incoming.fixup_all();
    let dives = incoming.dives.len();
    let sites = incoming.sites.len();
    let before = (state.log)();
    let mut after = before.clone();
    after.merge(incoming);
    state.dispatch(Command::Snapshot {
        label: i18n::t2(state.strings().undo_import, dives, sites),
        before: Box::new(before),
        after: Box::new(after),
    });
    (dives, sites)
}
