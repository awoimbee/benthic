use dioxus::prelude::*;

use crate::actions;
use crate::components::ImportExport;
use crate::state::AppState;

#[component]
pub fn Toolbar() -> Element {
    let state = use_context::<AppState>();
    let status = (state.status)();
    let log = (state.log)();
    let can_undo = state.can_undo();
    let can_redo = state.can_redo();
    let selected_count = (state.selection)().len();
    let mut filter = state.filter;
    let mut selection = state.selection;
    let mut show_prefs = state.show_prefs;
    let mut show_palette = state.show_palette;
    let mut show_planner = state.show_planner;
    let mut show_sync = state.show_sync;
    // On narrow screens the secondary actions fold into a dropdown.
    let mut menu_open = use_signal(|| false);
    let open = (menu_open)();

    rsx! {
        header { class: "toolbar",
            span { class: "brand", "benthic" }
            span { class: "muted toolbar-count", "{log.dives.len()} dives" }

            button {
                class: "btn primary",
                onclick: move |_| actions::new_dive(state),
                "+ New dive"
            }

            div { class: "spacer" }

            input {
                class: "search",
                r#type: "search",
                placeholder: "Search dives…",
                value: "{filter().query}",
                oninput: move |evt| filter.write().query = evt.value(),
            }

            button {
                class: "btn menu-toggle",
                title: "More actions",
                onclick: move |_| menu_open.set(!open),
                if open { "✕" } else { "☰" }
            }

            div {
                class: if open { "toolbar-actions open" } else { "toolbar-actions" },

                button {
                    class: if log.autogroup { "btn active" } else { "btn" },
                    title: "Automatically group nearby dives into trips",
                    onclick: move |_| actions::toggle_autogroup(state),
                    if log.autogroup { "Auto-group: on" } else { "Auto-group: off" }
                }
                button {
                    class: "btn",
                    title: "Rename, merge and delete trips",
                    onclick: move |_| actions::open_trips(state),
                    "Trips"
                }
                button {
                    class: "btn",
                    title: "Merge dive sites with the same name",
                    onclick: move |_| actions::merge_duplicate_sites(state),
                    "Merge sites"
                }

                if selected_count > 0 {
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
                        "Delete selected"
                    }
                    button {
                        class: "btn",
                        onclick: move |_| { selection.write().clear(); },
                        "Clear"
                    }
                }

                ImportExport {}

                button {
                    class: "btn",
                    disabled: !can_undo,
                    title: "Undo (Ctrl/Cmd+Z)",
                    onclick: move |_| state.undo(),
                    "Undo"
                }
                button {
                    class: "btn",
                    disabled: !can_redo,
                    title: "Redo (Ctrl/Cmd+Shift+Z)",
                    onclick: move |_| state.redo(),
                    "Redo"
                }
                button {
                    class: "btn",
                    title: "Preferences",
                    onclick: move |_| show_prefs.set(true),
                    "Preferences"
                }
                button {
                    class: "btn",
                    title: "Command palette (Ctrl/Cmd+K)",
                    onclick: move |_| show_palette.set(true),
                    "Commands"
                }
                button {
                    class: "btn",
                    title: "Bühlmann dive planner",
                    onclick: move |_| show_planner.set(true),
                    "Planner"
                }

                button {
                    class: "btn",
                    title: "Sync the log with a Git repository or Google Drive",
                    onclick: move |_| show_sync.set(true),
                    "Sync"
                }
                span { class: "status", "{status}" }
            }
        }
    }
}
