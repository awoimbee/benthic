use dioxus::prelude::*;

use crate::actions;
use crate::components::ImportExport;
use crate::i18n::{self, Strings};
use crate::state::AppState;

/// A top-level category in the header. On wide screens each is its own
/// dropdown; on phones they are stacked into one sheet with headings.
#[derive(Clone, Copy, PartialEq, Eq)]
enum MenuSection {
    Dives,
    Tools,
    Files,
    Settings,
}

impl MenuSection {
    const ALL: [MenuSection; 4] = [
        MenuSection::Dives,
        MenuSection::Tools,
        MenuSection::Files,
        MenuSection::Settings,
    ];

    fn label(self, t: &'static Strings) -> &'static str {
        match self {
            MenuSection::Dives => t.cat_dives,
            MenuSection::Tools => t.cat_tools,
            MenuSection::Files => t.cat_files,
            MenuSection::Settings => t.cat_settings,
        }
    }
}

/// The application header.
///
/// Only the primary actions stay visible: **New dive** and search. The rest is
/// grouped into a few named categories so it reads like a menu bar rather than
/// a wall of buttons.
#[component]
pub fn Toolbar() -> Element {
    let state = use_context::<AppState>();
    let t = i18n::strings((state.prefs)().language);
    let status = (state.status)();
    let log = (state.log)();
    let mut filter = state.filter;
    let dives_count = i18n::t1(t.dives_count, log.dives.len());

    let mut open_menu = use_signal(|| None::<MenuSection>);
    let mut sheet_open = use_signal(|| false);
    let open = (open_menu)();
    let sheet = (sheet_open)();

    // Close the menu as soon as a dialog opens, so it never lingers on top of
    // (or behind) the modal that replaces it.
    use_effect(move || {
        let dialog_open = (state.show_prefs)()
            || (state.show_trips)()
            || (state.show_map)()
            || (state.show_planner)()
            || (state.show_sync)()
            || (state.show_compare)()
            || (state.show_download)();
        if dialog_open {
            open_menu.set(None);
            sheet_open.set(false);
        }
    });

    rsx! {
        header { class: "toolbar",
            span { class: "brand", "benthic" }
            span { class: "muted toolbar-count", "{dives_count}" }

            button {
                class: "btn primary",
                onclick: move |_| actions::new_dive(state),
                "{t.new_dive}"
            }

            div { class: "spacer" }

            input {
                class: "search",
                r#type: "search",
                placeholder: "{t.search_placeholder}",
                value: "{filter().query}",
                oninput: move |evt| filter.write().query = evt.value(),
            }

            span {
                class: "status status-wide",
                role: "status",
                aria_live: "polite",
                "{status}"
            }

            // Menu bar: one dropdown per category on wide screens.
            div { class: "toolbar-menus",
                for section in MenuSection::ALL {
                    div { class: "menu",
                        button {
                            class: if open == Some(section) { "menu-trigger open" } else { "menu-trigger" },
                            aria_expanded: if open == Some(section) { "true" } else { "false" },
                            onclick: move |_| {
                                let mut open_menu = open_menu;
                                let next = if open_menu() == Some(section) {
                                    None
                                } else {
                                    Some(section)
                                };
                                open_menu.set(next);
                            },
                            "{section.label(t)} \u{25be}"
                        }
                        if open == Some(section) {
                            div {
                                class: "menu-panel",
                                onclick: move |_| {
                                    let mut open_menu = open_menu;
                                    open_menu.set(None);
                                },
                                MenuContent { sections: vec![section], show_headings: false }
                            }
                        }
                    }
                }
            }

            // Phones fold every category into one sheet.
            button {
                class: "btn menu-toggle",
                title: "{t.menu}",
                aria_expanded: if sheet { "true" } else { "false" },
                onclick: move |_| {
                    let mut sheet_open = sheet_open;
                    sheet_open.set(!sheet_open());
                },
                span { class: "menu-icon", if sheet { "✕" } else { "☰" } }
                span { class: "menu-label", "{t.menu}" }
            }

            if open.is_some() || sheet {
                div {
                    class: "menu-backdrop",
                    onclick: move |_| {
                        let mut open_menu = open_menu;
                        let mut sheet_open = sheet_open;
                        open_menu.set(None);
                        sheet_open.set(false);
                    },
                }
            }

            if sheet {
                div {
                    class: "toolbar-sheet",
                    onclick: move |_| {
                        let mut sheet_open = sheet_open;
                        sheet_open.set(false);
                    },
                    MenuContent { sections: MenuSection::ALL.to_vec(), show_headings: true }
                    div {
                        class: "menu-status",
                        role: "status",
                        aria_live: "polite",
                        "{status}"
                    }
                }
            }
        }
    }
}

/// The items for one or more categories. Kept as a component so the desktop
/// dropdowns and the phone sheet share exactly the same markup.
#[component]
fn MenuContent(sections: Vec<MenuSection>, show_headings: bool) -> Element {
    let state = use_context::<AppState>();
    let t = i18n::strings((state.prefs)().language);
    let log = (state.log)();
    let can_undo = state.can_undo();
    let can_redo = state.can_redo();
    let selected_count = (state.selection)().len();
    let mut selection = state.selection;
    let mut show_map = state.show_map;
    let mut show_sync = state.show_sync;

    rsx! {
        for section in sections {
            if show_headings {
                div { class: "menu-heading", "{section.label(t)}" }
            }
            {match section {
                MenuSection::Dives => rsx! {
                    if selected_count > 0 {
                        if selected_count == 2 {
                            MenuItem {
                                label: t.compare_dives.to_string(),
                                onclick: move |_| actions::open_compare(state),
                            }
                        }
                        MenuItem {
                            label: t.group_new_trip.to_string(),
                            onclick: move |_| actions::create_trip_from_selection(state),
                        }
                        MenuItem {
                            label: t.delete_selected.to_string(),
                            danger: true,
                            onclick: move |_| {
                                let mut confirm = state.confirm_delete_selected;
                                confirm.set(true);
                            },
                        }
                        MenuItem {
                            label: t.clear_selection.to_string(),
                            onclick: move |_| {
                                selection.write().clear();
                            },
                        }
                    }
                    MenuItem {
                        label: if log.autogroup {
                            t.autogroup_on.to_string()
                        } else {
                            t.autogroup_off.to_string()
                        },
                        active: log.autogroup,
                        onclick: move |_| actions::toggle_autogroup(state),
                    }
                    MenuItem {
                        label: t.manage_trips.to_string(),
                        onclick: move |_| actions::open_trips(state),
                    }
                    MenuItem {
                        label: t.merge_duplicate_sites.to_string(),
                        onclick: move |_| actions::merge_duplicate_sites(state),
                    }
                    MenuItem {
                        label: t.undo.to_string(),
                        disabled: !can_undo,
                        onclick: move |_| state.undo(),
                    }
                    MenuItem {
                        label: t.redo.to_string(),
                        disabled: !can_redo,
                        onclick: move |_| state.redo(),
                    }
                },
                MenuSection::Tools => rsx! {
                    MenuItem {
                        label: t.map.to_string(),
                        onclick: move |_| show_map.set(true),
                    }
                    MenuItem {
                        label: t.dive_planner.to_string(),
                        onclick: move |_| actions::open_planner(state),
                    }
                },
                MenuSection::Files => rsx! {
                    ImportExport {}
                    DownloadItem {}
                    MenuItem {
                        label: t.sync.to_string(),
                        onclick: move |_| show_sync.set(true),
                    }
                },
                MenuSection::Settings => rsx! {
                    MenuItem {
                        label: t.preferences.to_string(),
                        onclick: move |_| actions::open_preferences(state),
                    }
                    PrivacyItem {}
                },
            }}
        }
    }
}

/// "Download from dive computer", hidden on platforms without a transport.
#[component]
fn DownloadItem() -> Element {
    #[cfg(any(feature = "divecomputer", target_arch = "wasm32"))]
    {
        let state = use_context::<AppState>();
        let t = i18n::strings((state.prefs)().language);
        let mut show_download = state.show_download;
        if (state.download_available)() {
            return rsx! {
                button {
                    class: "menu-item",
                    r#type: "button",
                    onclick: move |_| show_download.set(true),
                    "{t.download_dc}"
                }
            };
        }
    }
    rsx! {}
}

/// The privacy-policy link, which only exists in the web bundle.
#[component]
fn PrivacyItem() -> Element {
    #[cfg(target_arch = "wasm32")]
    {
        let state = use_context::<AppState>();
        let t = i18n::strings((state.prefs)().language);
        rsx! {
            a {
                class: "menu-item",
                href: "privacy.html",
                target: "_blank",
                rel: "noopener",
                "{t.privacy_policy}"
            }
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        rsx! {}
    }
}

/// One row inside a menu.
#[component]
fn MenuItem(
    label: String,
    onclick: EventHandler<MouseEvent>,
    #[props(default)] disabled: bool,
    #[props(default)] danger: bool,
    #[props(default)] active: bool,
) -> Element {
    let mut class = String::from("menu-item");
    if danger {
        class.push_str(" danger");
    }
    if active {
        class.push_str(" active");
    }
    rsx! {
        button {
            class: "{class}",
            r#type: "button",
            disabled,
            onclick: move |evt| onclick.call(evt),
            "{label}"
        }
    }
}
