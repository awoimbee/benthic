//! Shared application state, provided to the component tree as context.

use dioxus::prelude::*;

use benthic_core::DiveLog;

/// Signals shared across the app. `Signal` is `Copy`, so this whole struct is
/// cheap to pass around and to provide as context.
#[derive(Clone, Copy)]
pub struct AppState {
    /// The entire dive log.
    pub log: Signal<DiveLog>,
    /// Currently selected dive id, if any.
    pub selected: Signal<Option<u32>>,
    /// A short human-readable status message shown in the toolbar.
    pub status: Signal<String>,
}

impl AppState {
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
