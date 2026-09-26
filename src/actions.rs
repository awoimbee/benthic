//! High-level user actions.
//!
//! Each action builds one or more [`Command`]s and dispatches them through the
//! [`AppState`] so that it is automatically undoable.

use benthic_core::{Command, Dive};
use dioxus::prelude::WritableExt;

use crate::state::AppState;

/// Days between consecutive dives before automatic grouping breaks a trip.
pub const AUTOGROUP_MAX_GAP_DAYS: i64 = 3;

/// Create an empty, manually-entered dive, select it, and open the editor.
pub fn new_dive(state: AppState) {
    let log = (state.log)();
    let mut dive = Dive::manual(crate::platform::now_secs());
    dive.id = log.next_id();
    dive.number = log.dives.iter().map(|d| d.number).max().unwrap_or(0) + 1;
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

/// Turn automatic trip grouping on/off, regrouping or ungrouping as needed.
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
