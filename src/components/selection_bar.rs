use dioxus::prelude::*;

use crate::actions;
use crate::state::AppState;

/// A thumb-reachable bottom bar with bulk actions, shown on narrow screens
/// while dives are ticked. On wide screens it is hidden by CSS; the toolbar
/// still carries the same actions there.
#[component]
pub fn SelectionBar() -> Element {
    let state = use_context::<AppState>();
    let mut selection = state.selection;
    let selected_count = (state.selection)().len();

    if selected_count == 0 {
        return rsx! {};
    }

    rsx! {
        div { class: "selection-bar",
            span { class: "selected-count", "{selected_count} selected" }
            if selected_count == 2 {
                button {
                    class: "btn",
                    title: "Compare the two selected dives",
                    onclick: move |_| actions::open_compare(state),
                    "Compare"
                }
            }
            button {
                class: "btn",
                title: "Group the selected dives into a new trip",
                onclick: move |_| actions::create_trip_from_selection(state),
                "New trip"
            }
            button {
                class: "btn danger",
                onclick: move |_| actions::delete_selected(state),
                "Delete"
            }
            button {
                class: "btn",
                onclick: move |_| {
                    selection.write().clear();
                },
                "Clear"
            }
        }
    }
}
