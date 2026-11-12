//! Per-dive selection before an imported file is merged or opened.

use std::collections::BTreeSet;

use dioxus::prelude::*;

use benthic_core::units::format_duration;
use benthic_core::DiveTrip;

use crate::actions;
use crate::i18n;
use crate::state::{AppState, ImportMode, PendingImport};

#[component]
pub fn ImportReviewDialog(pending: PendingImport) -> Element {
    let mut state = use_context::<AppState>();
    let prefs = (state.prefs)();
    let tr = i18n::strings(prefs.language);

    let initial_selected: BTreeSet<u32> = pending
        .log
        .dives
        .iter()
        .filter(|dive| !pending.duplicates.contains(&dive.id))
        .map(|dive| dive.id)
        .collect();
    let selected = use_signal(|| initial_selected);
    let mut add_trip = use_signal(|| false);
    let mut trip_name = use_signal(String::new);
    let selected_ids = (selected)();
    let selected_count = selected_ids.len();
    let total = pending.log.dives.len();
    let all_ids: BTreeSet<u32> = pending.log.dives.iter().map(|dive| dive.id).collect();

    let row_elements: Vec<Element> = pending
        .log
        .dives_recent_first()
        .into_iter()
        .map(|dive| {
            let id = dive.id;
            let checked = selected_ids.contains(&id);
            let duplicate = pending.duplicates.contains(&id);
            let date = prefs.timestamp(dive.when);
            let site = dive
                .site_id
                .and_then(|site_id| pending.log.site_by_uuid(site_id))
                .map(|site| site.name.clone())
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| "—".to_string());
            let duration = dive
                .duration()
                .map(format_duration)
                .unwrap_or_else(|| "—".to_string());
            let depth = dive
                .max_depth()
                .map(|depth| prefs.depth(depth))
                .unwrap_or_else(|| "—".to_string());
            let mut selected = selected;
            rsx! {
                label { key: "{id}", class: "import-review-row",
                    input {
                        r#type: "checkbox",
                        checked,
                        onchange: move |_| {
                            let mut set = selected.write();
                            if set.contains(&id) {
                                set.remove(&id);
                            } else {
                                set.insert(id);
                            }
                        },
                    }
                    span { class: "import-review-main",
                        span { class: "import-review-date", "{date}" }
                        span { class: "import-review-site", "{site}" }
                    }
                    span { class: "import-review-meta", "{duration}" }
                    span { class: "import-review-meta", "{depth}" }
                    if duplicate {
                        span { class: "import-review-badge", "{tr.already_in_log}" }
                    }
                }
            }
        })
        .collect();

    let confirm_pending = pending.clone();
    let selected_for_confirm = selected;
    let on_confirm = move |_| {
        let ids = (selected_for_confirm)();
        if ids.is_empty() {
            return;
        }
        let add_trip_flag = (add_trip)();
        let trip_name_value = (trip_name)();
        let mut import_review = state.import_review;
        let Some(mut pending) = import_review.take() else {
            return;
        };
        #[cfg(any(feature = "divecomputer", target_arch = "wasm32"))]
        let fingerprint = {
            let required: BTreeSet<u32> = pending
                .log
                .dives
                .iter()
                .filter(|dive| !pending.duplicates.contains(&dive.id))
                .map(|dive| dive.id)
                .collect();
            required
                .iter()
                .all(|id| ids.contains(id))
                .then(|| pending.fingerprint.clone())
                .flatten()
        };
        pending.log.dives.retain(|dive| ids.contains(&dive.id));
        if add_trip_flag && !pending.log.dives.is_empty() {
            let id = pending.log.next_trip_id();
            let date = pending.log.dives.iter().map(|dive| dive.when).min();
            let site = pending
                .log
                .dives
                .first()
                .and_then(|dive| dive.site_id)
                .and_then(|site_id| pending.log.site_by_uuid(site_id))
                .map(|site| site.name.clone())
                .filter(|name| !name.is_empty());
            let location = if trip_name_value.trim().is_empty() {
                site.unwrap_or_else(|| tr.imported_trip_name.to_string())
            } else {
                trip_name_value.trim().to_string()
            };
            pending.log.trips.push(DiveTrip {
                id,
                date,
                location,
                ..Default::default()
            });
            for dive in &mut pending.log.dives {
                dive.trip_id = Some(id);
            }
        }
        let file_name = pending.file_name.clone();
        let (dives, sites) = actions::merge_log(state, pending.log);
        if (state.selected)().is_none() {
            state.selected.set((state.log)().newest_dive_id());
        }
        #[cfg(any(feature = "divecomputer", target_arch = "wasm32"))]
        if let Some((key, value)) = fingerprint {
            crate::divecomputer::save_fingerprint(&key, &value);
        }
        match confirm_pending.mode {
            ImportMode::Merge => state.set_status(i18n::t3(tr.imported, dives, sites, file_name)),
            ImportMode::Download => {
                state.set_status(i18n::t1(tr.downloading_dc, dives));
            }
        }
    };

    let all_for_select = all_ids.clone();
    let mut selected_for_all = selected;
    let title = tr.import_review_title;
    let confirm_label = tr.import_selected;

    rsx! {
        div {
            class: "modal-backdrop",
            onclick: move |_| state.import_review.set(None),
            div {
                class: "modal wide",
                role: "dialog",
                tabindex: "-1",
                autofocus: true,
                aria_modal: "true",
                aria_label: "{title}",
                onclick: move |evt| evt.stop_propagation(),
                h2 { "{title}" }
                p { class: "muted",
                    {i18n::t2(tr.import_review_summary, selected_count, total)}
                    if !pending.duplicates.is_empty() {
                        " · "
                        {i18n::t1(tr.import_review_duplicates, pending.duplicates.len())}
                    }
                }
                label { class: "check import-review-trip-toggle",
                    input {
                        r#type: "checkbox",
                        checked: (add_trip)(),
                        onchange: move |_| add_trip.set(!(add_trip)()),
                    }
                    span { "{tr.import_create_trip}" }
                }
                if (add_trip)() {
                    input {
                        class: "field",
                        placeholder: "{tr.trip_name_placeholder}",
                        value: "{trip_name}",
                        oninput: move |evt| trip_name.set(evt.value()),
                    }
                }
                div { class: "import-review-actions",
                    button {
                        class: "btn",
                        r#type: "button",
                        onclick: move |_| selected_for_all.set(all_for_select.clone()),
                        "{tr.select_all}"
                    }
                    button {
                        class: "btn",
                        r#type: "button",
                        onclick: move |_| selected_for_all.set(BTreeSet::new()),
                        "{tr.select_none}"
                    }
                }
                div { class: "import-review-list",
                    for row in row_elements {
                        {row}
                    }
                }
                div { class: "detail-actions",
                    button {
                        class: "btn primary",
                        r#type: "button",
                        disabled: selected_count == 0,
                        onclick: on_confirm,
                        "{confirm_label}"
                    }
                    button {
                        class: "btn",
                        r#type: "button",
                        onclick: move |_| state.import_review.set(None),
                        "{tr.cancel}"
                    }
                }
            }
        }
    }
}
