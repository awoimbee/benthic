//! High-level user actions.
//!
//! Each action builds one or more [`Command`]s and dispatches them through the
//! [`AppState`] so that it is automatically undoable.

use benthic_core::{Command, Dive, DivePlan, DiveTrip};
use dioxus::prelude::WritableExt;

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
    state.set_status("Added a new dive");
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
    state.set_status("Duplicated dive");
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
    state.set_status("Deleted dive");
}

/// Delete every ticked dive as a single undo step.
pub fn delete_selected(state: AppState) {
    let ids: Vec<u32> = (state.selection)().iter().copied().collect();
    if ids.is_empty() {
        return;
    }
    let log = (state.log)();
    let commands = benthic_core::history::delete_dives(&log, &ids);
    state.dispatch_all(format!("Delete {} dives", ids.len()), commands);

    let mut selection = state.selection;
    selection.write().clear();

    let mut selected = state.selected;
    if selected().is_some_and(|id| ids.contains(&id)) {
        selected.set((state.log)().dives_sorted().first().map(|d| d.id));
    }
    state.set_status(format!("Deleted {} dives", ids.len()));
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
    state.dispatch_all("Create trip", commands);
    let mut selection = state.selection;
    selection.write().clear();
    state.set_status(format!("Created a trip with {count} dives"));
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
    state.set_status("Renamed trip");
}

/// Delete a trip, unassigning its dives as one undo step.
pub fn delete_trip(state: AppState, id: u32) {
    let log = (state.log)();
    let commands = benthic_core::history::delete_trip(&log, id);
    if commands.is_empty() {
        return;
    }
    state.dispatch_all("Delete trip", commands);
    state.set_status("Deleted trip");
}

/// Merge dive sites that share a name (and are close, or lack coordinates).
pub fn merge_duplicate_sites(state: AppState) {
    let before = (state.log)();
    let mut after = before.clone();
    let removed = after.merge_duplicate_sites(SITE_DEDUP_RADIUS_M);
    if removed == 0 {
        state.set_status("No duplicate dive sites found");
        return;
    }
    state.dispatch(Command::Snapshot {
        label: "Merge duplicate sites".into(),
        before: Box::new(before),
        after: Box::new(after),
    });
    state.set_status(format!("Merged {removed} duplicate dive sites"));
}

/// Restore the most recent automatic backup over the current log.
pub fn restore_backup(state: AppState) {
    let Some(text) = crate::storage::load_backup() else {
        state.set_status("No automatic backup available");
        return;
    };
    match benthic_core::io::parse_auto(&text) {
        Ok(parsed) => {
            let before = (state.log)();
            state.dispatch(Command::Snapshot {
                label: "Restore backup".into(),
                before: Box::new(before),
                after: Box::new(parsed),
            });
            state.set_status("Restored the last automatic backup");
        }
        Err(e) => state.set_status(format!("Backup could not be read: {e}")),
    }
}

/// Export the log as Subsurface XML (browser download or file write).
pub fn export_ssrf(state: AppState) {
    let log = (state.log)();
    let text = benthic_core::io::ssrf::write_string(&log);
    match crate::platform::save_file("benthic.ssrf", &text) {
        Ok(message) => state.set_status(message),
        Err(e) => state.set_status(format!("Export failed: {e}")),
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
    state.set_status("Saved plan as a dive");
}

/// Merge the `remove` trip into the `keep` trip as one undo step.
pub fn merge_trips(state: AppState, keep: u32, remove: u32) {
    let before = (state.log)();
    let mut after = before.clone();
    if !after.merge_trips(keep, remove) {
        return;
    }
    state.dispatch(Command::Snapshot {
        label: "Merge trips".into(),
        before: Box::new(before),
        after: Box::new(after),
    });
    state.set_status("Merged trips");
}

/// Toggle automatic trip grouping on/off, regrouping or ungrouping as needed.
pub fn toggle_autogroup(state: AppState) {
    let mut after = (state.log)();
    let before = after.clone();
    let (label, message) = if after.autogroup {
        after.autogroup = false;
        after.clear_auto_trips();
        ("Ungroup trips", "Automatic trip grouping off".to_string())
    } else {
        after.autogroup = true;
        let created = after.autogroup_trips(AUTOGROUP_MAX_GAP_DAYS);
        ("Auto-group trips", format!("Created {created} trips"))
    };
    state.dispatch(Command::Snapshot {
        label: label.into(),
        before: Box::new(before),
        after: Box::new(after),
    });
    state.set_status(message);
}

/// Merge an imported log into the current one as a single undoable step.
pub fn merge_log(state: AppState, mut incoming: benthic_core::DiveLog) -> usize {
    incoming.fixup_all();
    let count = incoming.dives.len();
    let before = (state.log)();
    let mut after = before.clone();
    after.merge(incoming);
    state.dispatch(Command::Snapshot {
        label: format!("Import {count} dives"),
        before: Box::new(before),
        after: Box::new(after),
    });
    count
}
