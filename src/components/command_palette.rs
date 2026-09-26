use dioxus::prelude::*;

use crate::actions;
use crate::state::AppState;

/// A keyboard-driven command palette: run an action or jump to a dive.
#[component]
pub fn CommandPalette() -> Element {
    let state = use_context::<AppState>();
    let mut show_palette = state.show_palette;
    let query = use_signal(String::new);
    let mut query_signal = query;

    let prefs = (state.prefs)();
    let log = (state.log)();
    let needle = (query)().trim().to_lowercase();

    let mut items: Vec<Item> = vec![
        Item::new("New dive", "Create an empty dive", Action::NewDive),
        Item::new("Undo", "Ctrl/Cmd+Z", Action::Undo),
        Item::new("Redo", "Ctrl/Cmd+Shift+Z", Action::Redo),
        Item::new(
            "Toggle auto-group",
            "Group nearby dives into trips",
            Action::AutoGroup,
        ),
        Item::new(
            "Merge duplicate sites",
            "Clean up repeated site names",
            Action::MergeSites,
        ),
        Item::new(
            "Create trip from selection",
            "Group ticked dives",
            Action::CreateTrip,
        ),
        Item::new(
            "Manage trips",
            "Rename, merge and delete trips",
            Action::ManageTrips,
        ),
        Item::new(
            "Delete selected",
            "Remove ticked dives",
            Action::DeleteSelected,
        ),
        Item::new(
            "Export .ssrf",
            "Download or write a Subsurface file",
            Action::Export,
        ),
        Item::new(
            "Preferences",
            "Units, formats and backups",
            Action::Preferences,
        ),
        Item::new("Dive planner", "Bühlmann NDL and ceiling", Action::Planner),
        Item::new(
            "Compare dives",
            "Overlay the two selected dives",
            Action::Compare,
        ),
    ];
    for dive in log.dives_recent_first() {
        items.push(Item::new(
            format!("Open: {}", crate::format::dive_title(dive, &log)),
            crate::format::dive_subtitle(dive, &prefs),
            Action::SelectDive(dive.id),
        ));
    }

    let filtered: Vec<Item> = items
        .into_iter()
        .filter(|item| needle.is_empty() || item.search().contains(&needle))
        .take(12)
        .collect();

    rsx! {
        div {
            class: "modal-backdrop",
            onclick: move |_| show_palette.set(false),
            div {
                class: "palette",
                onclick: move |evt| evt.stop_propagation(),
                input {
                    class: "palette-input",
                    autofocus: true,
                    placeholder: "Type a command or dive…",
                    value: "{query()}",
                    oninput: move |evt| query_signal.set(evt.value()),
                    onkeydown: move |evt| {
                        if evt.key() == Key::Escape {
                            show_palette.set(false);
                        }
                    },
                }
                if filtered.is_empty() {
                    div { class: "empty-hint", "No matching commands." }
                }
                ul { class: "palette-list",
                    for (index, item) in filtered.into_iter().enumerate() {
                        li {
                            key: "{index}",
                            class: "palette-item",
                            onclick: move |_| {
                                run(state, item.action.clone());
                                show_palette.set(false);
                            },
                            span { class: "palette-label", "{item.label}" }
                            span { class: "palette-hint", "{item.hint}" }
                        }
                    }
                }
            }
        }
    }
}

#[derive(Clone, PartialEq)]
enum Action {
    NewDive,
    Undo,
    Redo,
    AutoGroup,
    MergeSites,
    CreateTrip,
    ManageTrips,
    DeleteSelected,
    Export,
    Preferences,
    Planner,
    Compare,
    SelectDive(u32),
}

struct Item {
    label: String,
    hint: String,
    action: Action,
}

impl Item {
    fn new(label: impl Into<String>, hint: impl Into<String>, action: Action) -> Self {
        Self {
            label: label.into(),
            hint: hint.into(),
            action,
        }
    }

    fn search(&self) -> String {
        format!("{} {}", self.label, self.hint).to_lowercase()
    }
}

fn run(state: AppState, action: Action) {
    match action {
        Action::NewDive => actions::new_dive(state),
        Action::Undo => state.undo(),
        Action::Redo => state.redo(),
        Action::AutoGroup => actions::toggle_autogroup(state),
        Action::MergeSites => actions::merge_duplicate_sites(state),
        Action::CreateTrip => actions::create_trip_from_selection(state),
        Action::ManageTrips => actions::open_trips(state),
        Action::DeleteSelected => actions::delete_selected(state),
        Action::Export => actions::export_ssrf(state),
        Action::Preferences => actions::open_preferences(state),
        Action::Planner => actions::open_planner(state),
        Action::Compare => actions::open_compare(state),
        Action::SelectDive(id) => {
            let mut selected = state.selected;
            selected.set(Some(id));
        }
    }
}
