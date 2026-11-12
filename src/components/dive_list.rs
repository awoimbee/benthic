use dioxus::prelude::*;

use crate::actions;
use crate::i18n;
use crate::state::AppState;

/// One row in the dive list; either a trip header or a dive.
struct Row {
    key: String,
    class: String,
    is_trip: bool,
    collapsed: bool,
    checked: bool,
    is_selected: bool,
    dive_id: Option<u32>,
    trip_id: Option<u32>,
    number: String,
    title: String,
    subtitle: String,
    trailing: String,
    freedive_group: bool,
    expanded: bool,
    day: Option<String>,
}

#[component]
pub fn DiveList() -> Element {
    let state = use_context::<AppState>();
    let mut selected = state.selected;
    let mut selection = state.selection;
    let mut mobile_detail = state.mobile_detail;
    let log = (state.log)();
    let filter = (state.filter)();
    let prefs = (state.prefs)();
    let t = i18n::strings(prefs.language);
    let current = (state.selected)();
    let checked_ids = (state.selection)();
    let mut renaming_trip = use_signal(|| None::<u32>);
    let mut rename_text = use_signal(String::new);
    let renaming = (renaming_trip)();
    // Trip ids whose dives are rolled up.
    let mut collapsed_trips = use_signal(std::collections::HashSet::<u32>::new);
    let collapsed = (collapsed_trips)();
    let mut expanded_freedives = use_signal(std::collections::HashSet::<String>::new);
    let expanded_days = (expanded_freedives)();

    let filtered: Vec<&benthic_core::Dive> = log
        .dives_recent_first()
        .into_iter()
        .filter(|dive| filter.matches(dive, &log))
        .collect();

    // Freedives are grouped by calendar day: the list shows one row per day
    // that contains several freedives, and expanding it reveals the individual
    // dives so one can still be selected.
    let mut freedive_days: std::collections::BTreeMap<String, Vec<&benthic_core::Dive>> =
        Default::default();
    for dive in &filtered {
        if let Some(day) = freedive_day(dive) {
            freedive_days.entry(day).or_default().push(dive);
        }
    }
    let grouped_days: std::collections::HashSet<String> = freedive_days
        .iter()
        .filter(|(_, dives)| dives.len() > 1)
        .map(|(day, _)| day.clone())
        .collect();
    let mut seen_freedive_days: std::collections::HashSet<String> = Default::default();

    let make_dive_row = |dive: &benthic_core::Dive, extra_class: &str| -> Row {
        Row {
            key: dive.id.to_string(),
            class: {
                let mut class = if current == Some(dive.id) {
                    String::from("dive-row selected")
                } else {
                    String::from("dive-row")
                };
                if !extra_class.is_empty() {
                    class.push(' ');
                    class.push_str(extra_class);
                }
                class
            },
            is_trip: false,
            collapsed: false,
            checked: checked_ids.contains(&dive.id),
            is_selected: current == Some(dive.id),
            dive_id: Some(dive.id),
            trip_id: None,
            number: if dive.number != 0 {
                dive.number.to_string()
            } else {
                "•".to_string()
            },
            title: crate::format::dive_title(dive, &log),
            subtitle: crate::format::dive_subtitle(dive, &prefs),
            trailing: dive
                .cylinders
                .first()
                .map(|c| c.gas.name())
                .unwrap_or_else(|| "—".to_string()),
            freedive_group: false,
            expanded: false,
            day: None,
        }
    };

    let mut rows: Vec<Row> = Vec::new();
    let mut seen_trips: std::collections::HashSet<u32> = Default::default();
    for (index, dive) in filtered.iter().enumerate() {
        let day = freedive_day(dive);
        if let Some(day) = day.as_ref().filter(|day| grouped_days.contains(*day)) {
            if seen_freedive_days.insert(day.clone()) {
                let dives = &freedive_days[day];
                let titles: Vec<String> = dives
                    .iter()
                    .map(|dive| crate::format::dive_title(dive, &log))
                    .collect();
                let title = if titles.iter().all(|title| title == &titles[0]) {
                    titles[0].clone()
                } else {
                    prefs.date(dives[0].when)
                };
                let date = prefs.date(dives[0].when);
                let title = if title == date {
                    title
                } else {
                    format!("{title} · {date}")
                };
                let expanded = expanded_days.contains(day);
                rows.push(Row {
                    key: format!("freedive-day-{day}"),
                    class: "trip-row freedive-day-row".to_string(),
                    is_trip: false,
                    collapsed: !expanded,
                    checked: false,
                    is_selected: false,
                    dive_id: None,
                    trip_id: None,
                    number: String::new(),
                    title,
                    subtitle: i18n::t1(t.n_freedives, dives.len()),
                    trailing: String::new(),
                    freedive_group: true,
                    expanded,
                    day: Some(day.clone()),
                });
                if expanded {
                    for freedive in dives {
                        rows.push(make_dive_row(freedive, "freedive-child"));
                    }
                }
            }
            continue;
        }

        // The last dive of a trip is marked so the group reads as one block
        // and is visibly separated from the dives that follow.
        let trip_end = dive.trip_id.is_some()
            && filtered.get(index + 1).map(|next| next.trip_id) != Some(dive.trip_id);
        if let Some(trip_id) = dive.trip_id {
            if seen_trips.insert(trip_id) {
                if let Some(trip) = log.trip_by_id(trip_id) {
                    let count = filtered
                        .iter()
                        .filter(|d| d.trip_id == Some(trip_id))
                        .count();
                    let title = if trip.location.is_empty() {
                        i18n::t1(t.trip_fallback, trip_id)
                    } else {
                        trip.location.clone()
                    };
                    rows.push(Row {
                        key: format!("trip-{trip_id}"),
                        class: "trip-row".to_string(),
                        is_trip: true,
                        collapsed: collapsed.contains(&trip_id),
                        checked: false,
                        is_selected: false,
                        dive_id: None,
                        trip_id: Some(trip_id),
                        number: String::new(),
                        title,
                        subtitle: i18n::t1(t.n_dives, count),
                        trailing: String::new(),
                        freedive_group: false,
                        expanded: false,
                        day: None,
                    });
                }
            }
        }
        if dive
            .trip_id
            .is_some_and(|trip_id| collapsed.contains(&trip_id))
        {
            continue;
        }
        let mut row = make_dive_row(dive, "");
        if dive.trip_id.is_some() {
            row.class.push_str(" in-trip");
        }
        if trip_end {
            row.class.push_str(" trip-end");
        }
        rows.push(row);
    }

    let total = log.dives.len();
    let shown = filtered.len();
    let empty = rows.is_empty();
    let dives_shown = i18n::t2(t.dives_shown, shown, total);

    rsx! {
        aside { class: "dive-list",
            div { class: "pane-title", "{dives_shown}" }
            if empty {
                div { class: "empty-hint",
                    if total == 0 { "{t.empty_no_dives}" } else { "{t.empty_no_match}" }
                }
            }
            ul {
                for row in rows {
                    li {
                        key: "{row.key}",
                        class: "{row.class}",
                        onclick: move |_| {
                            // Dive rows select on a click anywhere; trip rows use
                            // their explicit toggle button.
                            if let Some(id) = row.dive_id {
                                selected.set(Some(id));
                                let mut show_trip = state.show_trip;
                                show_trip.set(None);
                                mobile_detail.set(true);
                            }
                        },
                        if row.freedive_group {
                            button {
                                class: "trip-toggle",
                                r#type: "button",
                                aria_expanded: if row.expanded { "true" } else { "false" },
                                onclick: move |_| {
                                    if let Some(day) = row.day.clone() {
                                        let mut set = expanded_freedives.write();
                                        if !set.remove(&day) {
                                            set.insert(day);
                                        }
                                    }
                                },
                                span { class: "trip-caret",
                                    if row.expanded { "\u{25BE}" } else { "\u{25B8}" }
                                }
                                span { class: "trip-title", "{row.title}" }
                                span { class: "trip-subtitle", "{row.subtitle}" }
                            }
                        } else if row.is_trip {
                            if let Some(trip_id) = row.trip_id {
                                if renaming == Some(trip_id) {
                                    input {
                                        class: "trip-rename",
                                        value: "{rename_text()}",
                                        onclick: move |evt| evt.stop_propagation(),
                                        oninput: move |evt| rename_text.set(evt.value()),
                                        onkeydown: move |evt| {
                                            if evt.key() == Key::Enter {
                                                actions::rename_trip(state, trip_id, &rename_text());
                                                renaming_trip.set(None);
                                            } else if evt.key() == Key::Escape {
                                                renaming_trip.set(None);
                                            }
                                        },
                                        onblur: move |_| {
                                            if renaming_trip() == Some(trip_id) {
                                                actions::rename_trip(state, trip_id, &rename_text());
                                                renaming_trip.set(None);
                                            }
                                        },
                                    }
                                } else {
                                    button {
                                        class: "trip-caret-btn",
                                        r#type: "button",
                                        aria_expanded: if row.collapsed { "false" } else { "true" },
                                        onclick: move |_| {
                                            if let Some(trip_id) = row.trip_id {
                                                let mut set = collapsed_trips.write();
                                                if !set.remove(&trip_id) {
                                                    set.insert(trip_id);
                                                }
                                            }
                                        },
                                        span { class: "trip-caret",
                                            if row.collapsed { "\u{25B8}" } else { "\u{25BE}" }
                                        }
                                    }
                                    button {
                                        class: "trip-toggle",
                                        r#type: "button",
                                        onclick: move |_| {
                                            if let Some(trip_id) = row.trip_id {
                                                let mut show_trip = state.show_trip;
                                                let mut mobile_detail = state.mobile_detail;
                                                show_trip.set(Some(trip_id));
                                                mobile_detail.set(true);
                                            }
                                        },
                                        span { class: "trip-title", "{row.title}" }
                                        span { class: "trip-subtitle", "{row.subtitle}" }
                                    }
                                    div { class: "trip-actions",
                                        button {
                                            class: "icon-btn",
                                            title: "{t.rename_trip}",
                                            onclick: move |evt| {
                                                evt.stop_propagation();
                                                let name = (state.log)()
                                                    .trip_by_id(trip_id)
                                                    .map(|t| t.location.clone())
                                                    .unwrap_or_default();
                                                rename_text.set(name);
                                                renaming_trip.set(Some(trip_id));
                                            },
                                            "✎"
                                        }
                                        button {
                                            class: "icon-btn",
                                            title: "{t.delete_trip}",
                                            onclick: move |evt| {
                                                evt.stop_propagation();
                                                actions::delete_trip(state, trip_id);
                                            },
                                            "✕"
                                        }
                                    }
                                }
                            }
                        } else {
                            input {
                                r#type: "checkbox",
                                class: "row-check",
                                aria_label: i18n::t1(t.select_dive_label, &row.title),
                                checked: row.checked,
                                onclick: move |evt| evt.stop_propagation(),
                                onchange: move |_| {
                                    if let Some(id) = row.dive_id {
                                        let mut set = selection.write();
                                        if set.contains(&id) {
                                            set.remove(&id);
                                        } else {
                                            set.insert(id);
                                        }
                                    }
                                },
                            }
                            span { class: "dive-number", "{row.number}" }
                            button {
                                class: "dive-main",
                                r#type: "button",
                                aria_current: if row.is_selected { "true" } else { "false" },
                                onclick: move |_| {
                                    if let Some(id) = row.dive_id {
                                        selected.set(Some(id));
                                        let mut show_trip = state.show_trip;
                                        show_trip.set(None);
                                        mobile_detail.set(true);
                                    }
                                },
                                span { class: "dive-title", "{row.title}" }
                                span { class: "dive-subtitle", "{row.subtitle}" }
                            }
                            span { class: "dive-gas", "{row.trailing}" }
                        }
                    }
                }
            }
        }
    }
}

/// The UTC calendar day (`YYYY-MM-DD`) of a freedive, used to group a day's
/// freedives into one row; `None` for other dives.
fn freedive_day(dive: &benthic_core::Dive) -> Option<String> {
    dive.is_freedive().then(|| {
        let stamp = benthic_core::format_timestamp_utc(dive.when);
        stamp.split(' ').next().unwrap_or("").to_string()
    })
}
