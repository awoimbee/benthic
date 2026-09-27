//! Shared application state, provided to the component tree as context.

use dioxus::prelude::*;

use benthic_core::{Command, DiveFilter, DiveLog, FilterPreset, History, Preferences};

/// Signals shared across the app. `Signal` is `Copy`, so this whole struct is
/// cheap to pass around and to provide as context.
#[derive(Clone, Copy)]
pub struct AppState {
    /// The entire dive log.
    pub log: Signal<DiveLog>,
    /// Currently selected dive id, if any.
    pub selected: Signal<Option<u32>>,
    /// Dives ticked for bulk actions.
    pub selection: Signal<std::collections::BTreeSet<u32>>,
    /// A short human-readable status message shown in the toolbar.
    pub status: Signal<String>,
    /// Undo/redo stack.
    pub history: Signal<History>,
    /// Active dive-list filter.
    pub filter: Signal<DiveFilter>,
    /// Saved filter presets.
    pub presets: Signal<Vec<FilterPreset>>,
    /// Display preferences (units, ...).
    pub prefs: Signal<Preferences>,
    /// Whether the preferences dialog is open.
    pub show_prefs: Signal<bool>,
    /// Whether the command palette is open.
    pub show_palette: Signal<bool>,
    /// Whether the trips manager is open.
    pub show_trips: Signal<bool>,
    /// Whether the dive planner is open.
    pub show_planner: Signal<bool>,
    /// Whether the two-dive comparison is open.
    pub show_compare: Signal<bool>,
    /// Narrow screens only: whether the detail screen is showing rather than
    /// the dive list. Ignored on wide screens, where both panes are visible.
    pub mobile_detail: Signal<bool>,
}

impl AppState {
    /// Apply a single command and record it for undo.
    pub fn dispatch(&self, command: Command) {
        let mut log = self.log;
        let mut history = self.history;
        let mut snapshot = log();
        let mut pending = history();
        pending.record(command, &mut snapshot);
        log.set(snapshot);
        history.set(pending);
    }

    /// Apply several commands as one undo step.
    pub fn dispatch_all(&self, label: impl Into<String>, mut commands: Vec<Command>) {
        match commands.len() {
            0 => {}
            1 => self.dispatch(commands.pop().expect("length checked")),
            _ => self.dispatch(Command::Compound {
                label: label.into(),
                commands,
            }),
        }
    }

    /// Undo the most recent command, if any.
    pub fn undo(&self) {
        if !self.can_undo() {
            return;
        }
        let mut log = self.log;
        let mut history = self.history;
        let mut status = self.status;
        let mut snapshot = log();
        let mut pending = history();
        if let Some(label) = pending.undo(&mut snapshot) {
            log.set(snapshot);
            history.set(pending);
            status.set(format!("Undid: {label}"));
        }
    }

    /// Redo the most recently undone command, if any.
    pub fn redo(&self) {
        if !self.can_redo() {
            return;
        }
        let mut log = self.log;
        let mut history = self.history;
        let mut status = self.status;
        let mut snapshot = log();
        let mut pending = history();
        if let Some(label) = pending.redo(&mut snapshot) {
            log.set(snapshot);
            history.set(pending);
            status.set(format!("Redid: {label}"));
        }
    }

    pub fn can_undo(&self) -> bool {
        (self.history)().can_undo()
    }

    pub fn can_redo(&self) -> bool {
        (self.history)().can_redo()
    }

    /// Snapshot of the currently selected dive.
    #[allow(dead_code)]
    pub fn selected_dive(&self) -> Option<benthic_core::Dive> {
        let id = (self.selected)()?;
        (self.log)().dive_by_id(id).cloned()
    }

    /// Replace the status message.
    #[allow(dead_code)]
    pub fn set_status(&self, message: impl Into<String>) {
        let mut status = self.status;
        status.set(message.into());
    }
}
