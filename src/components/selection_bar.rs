use dioxus::prelude::*;

use crate::actions;
use crate::i18n;
use crate::state::AppState;

/// A thumb-reachable bottom bar with bulk actions, shown on narrow screens
/// while dives are ticked. On wide screens it is hidden by CSS; the toolbar
/// still carries the same actions there.
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
                class: "btn",
                onclick: move |_| actions::create_trip_from_selection(state),
                "{t.group_new_trip}"
            }
            button {
                class: "btn danger",
                onclick: move |_| {
                    let mut confirm = state.confirm_delete_selected;
                    confirm.set(true);
                },
                "{t.delete}"
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
