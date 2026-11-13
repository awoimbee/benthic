//! Shared application state, provided to the component tree as context.

use dioxus::prelude::*;

use benthic_core::{Command, DiveFilter, DiveLog, FilterPreset, History, Preferences};

use crate::i18n::{self, Strings};
use crate::routing::Route;

/// How a reviewed import should be applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportMode {
    Merge,
    #[allow(dead_code)]
    Download,
}

/// A modal dialog. Dialogs with a slug also have a URL (see `routing`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dialog {
    Preferences,
    Trips,
    Map,
    People,
    Planner,
    Sync,
    Download,
    Compare,
    BatchEdit,
    Renumber,
    ConfirmDeleteSelected,
}

impl Dialog {
    pub const ROUTABLE: [Dialog; 8] = [
        Dialog::Preferences,
        Dialog::Trips,
        Dialog::Map,
        Dialog::People,
        Dialog::Planner,
        Dialog::Sync,
        Dialog::Download,
        Dialog::Compare,
    ];

    /// The URL path segment, for dialogs that have an address.
    pub fn slug(self) -> Option<&'static str> {
        Some(match self {
            Dialog::Preferences => "preferences",
            Dialog::Trips => "trips",
            Dialog::Map => "map",
            Dialog::People => "people",
            Dialog::Planner => "planner",
            Dialog::Sync => "sync",
            Dialog::Download => "download",
            Dialog::Compare => "compare",
            Dialog::BatchEdit | Dialog::Renumber | Dialog::ConfirmDeleteSelected => return None,
        })
    }

    pub fn from_slug(slug: &str) -> Option<Dialog> {
        Dialog::ROUTABLE
            .into_iter()
            .find(|dialog| dialog.slug() == Some(slug))
    }
}

/// A parsed file waiting for the user to choose which dives to import.
#[derive(Clone, PartialEq)]
pub struct PendingImport {
    pub log: DiveLog,
    pub file_name: String,
    pub mode: ImportMode,
    /// Incoming dive ids that already exist in the current log.
    pub duplicates: std::collections::BTreeSet<u32>,
    /// Device fingerprint to remember only after a complete review.
    pub fingerprint: Option<(String, Vec<u8>)>,
    pub nonce: u64,
}

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
    /// Whether the structured filter fields are expanded.
    pub show_filters: Signal<bool>,
    /// Display preferences (units, ...).
    pub prefs: Signal<Preferences>,
    /// The open modal dialog, if any. Only one is open at a time.
    pub dialog: Signal<Option<Dialog>>,
    /// Whether the assistant panel is open beside the dive panes.
    pub show_ai: Signal<bool>,
    /// Coordinates the site map should centre on when it opens, if any.
    pub map_focus: Signal<Option<(f64, f64)>>,
    /// Whether this platform can talk to a dive computer at all (Web
    /// Serial/Bluetooth detected, or a native build). Drives whether the
    /// Download button is offered.
    #[allow(dead_code)]
    pub download_available: Signal<bool>,
    /// A parsed file awaiting per-dive import selection.
    pub import_review: Signal<Option<PendingImport>>,
    /// The trip currently shown in the trip view, if any.
    pub show_trip: Signal<Option<u32>>,
    /// Narrow screens only: whether the detail screen is showing rather than
    /// the dive list. Ignored on wide screens, where both panes are visible.
    pub mobile_detail: Signal<bool>,
    /// The current URL-backed section.
    pub route: Signal<Route>,
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
        self.step_history(History::undo, self.strings().undid);
    }

    /// Redo the most recently undone command, if any.
    pub fn redo(&self) {
        self.step_history(History::redo, self.strings().redid);
    }

    fn step_history(&self, step: fn(&mut History, &mut DiveLog) -> Option<String>, template: &str) {
        let mut log = self.log;
        let mut history = self.history;
        let mut snapshot = log();
        let mut pending = history();
        if let Some(label) = step(&mut pending, &mut snapshot) {
            log.set(snapshot);
            history.set(pending);
            self.set_status(i18n::t1(template, label));
        }
    }

    pub fn can_undo(&self) -> bool {
        (self.history)().can_undo()
    }

    pub fn can_redo(&self) -> bool {
        (self.history)().can_redo()
    }

    /// Open `dialog`, replacing any open one.
    pub fn open(&self, dialog: Dialog) {
        let mut open = self.dialog;
        open.set(Some(dialog));
    }

    /// Close the open dialog.
    pub fn close_dialog(&self) {
        let mut open = self.dialog;
        open.set(None);
    }

    /// Replace the status message.
    pub fn set_status(&self, message: impl Into<String>) {
        let mut status = self.status;
        status.set(message.into());
    }

    /// The translation table for the active language.
    pub fn strings(&self) -> &'static Strings {
        i18n::strings((self.prefs)().language)
    }
}
