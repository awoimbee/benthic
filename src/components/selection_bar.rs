use dioxus::prelude::*;

use crate::actions;
use crate::i18n;
use crate::state::{AppState, Dialog};

/// The bulk-action bar shown while dives are ticked: a thumb-reachable bottom
/// bar on phones and a floating bar on wide screens. Keeping the actions here
/// makes them discoverable without opening a menu.
#[component]
pub fn SelectionBar() -> Element {
    let state = use_context::<AppState>();
    let t = state.strings();
    let mut selection = state.selection;
    let selected_count = (state.selection)().len();

    if selected_count == 0 {
        return rsx! {};
    }

    rsx! {
        div { class: "selection-bar",
            span { class: "selected-count", {i18n::t1(t.selected_count, selected_count)} }
            if selected_count == 2 {
                button {
                    class: "btn",
                    onclick: move |_| actions::open_compare(state),
                    "{t.compare_dives}"
                }
            }
            button {
                class: "btn primary",
                onclick: move |_| actions::open_batch_edit(state),
                "{t.edit_selection}"
            }
            button {
                class: "btn",
                onclick: move |_| actions::create_trip_from_selection(state),
                "{t.group_new_trip}"
            }
            button {
                class: "btn danger",
                onclick: move |_| {
                    state.open(Dialog::ConfirmDeleteSelected);
                },
                {i18n::t1(t.delete_selected_count, selected_count)}
            }
            button {
                class: "btn",
                onclick: move |_| {
                    selection.write().clear();
                },
                "{t.clear}"
            }
        }
    }
}
