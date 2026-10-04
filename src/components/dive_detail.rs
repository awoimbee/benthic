use dioxus::prelude::*;

use benthic_core::equipment::{apply_preset, cylinder_preset, is_preset, CYLINDER_PRESETS};
use benthic_core::units::{format_duration, Timestamp, Weight};
use benthic_core::{Command, Cylinder, CylinderUse, Dive, DiveSite, Location, WeightSystem};

use crate::actions;
use crate::components::{
    profile_bounds, DiveProfile, LocationPickerDialog, MapSite, MapView, ProfileEditorDialog,
};
use crate::i18n;
use crate::state::AppState;

#[component]
pub fn DiveDetail() -> Element {
    let state = use_context::<AppState>();
    let tr = i18n::strings((state.prefs)().language);
    let selected = (state.selected)();
    let log = (state.log)();

    let Some(dive) = selected.and_then(|id| log.dive_by_id(id).cloned()) else {
        return rsx! {
            section { class: "detail",
                div { class: "empty-hint", "{tr.select_dive}" }
            }
        };
    };

    // Remount on selection change so the edit form resets.
    rsx! { DiveDetailInner { key: "{dive.id}", dive } }
}

/// The editable subset of a dive, kept separate from the full [`Dive`] so that
/// typing does not clone the (potentially huge) sample arrays.
#[derive(Clone, PartialEq)]
struct DiveForm {
    when: Timestamp,
    number: i32,
    buddy: String,
    divemaster: String,
    suit: String,
    notes: String,
    rating: u8,
    tags: String,
    trip_id: Option<u32>,
    site_name: String,
    site_gps: String,
    cylinders: Vec<Cylinder>,
    weights: Vec<WeightSystem>,
}

impl DiveForm {
    fn from_dive(dive: &Dive, site: Option<&DiveSite>) -> Self {
        Self {
            when: dive.when,
            number: dive.number,
            buddy: dive.buddy.clone(),
            divemaster: dive.diveguide.clone(),
            suit: dive.suit.clone(),
            notes: dive.notes.clone(),
            rating: dive.rating,
            tags: dive.tags.join(", "),
            trip_id: dive.trip_id,
            site_name: site.map(|s| s.name.clone()).unwrap_or_default(),
            site_gps: site
                .and_then(|s| s.location)
                .map(|l| format!("{:.6}, {:.6}", l.lat, l.lon))
                .unwrap_or_default(),
            cylinders: dive.cylinders.clone(),
            weights: dive.weights.clone(),
        }
    }
}

#[component]
fn DiveDetailInner(dive: Dive) -> Element {
    let state = use_context::<AppState>();
    let mut mobile_detail = state.mobile_detail;
    let log = (state.log)();
    let prefs = (state.prefs)();
    let tr = i18n::strings(prefs.language);
    let site = dive.site_id.and_then(|id| log.site_by_uuid(id)).cloned();

    let mut editing = use_signal(|| false);
    let mut form = use_signal(|| DiveForm::from_dive(&dive, site.as_ref()));
    // Samples edited in the profile editor, applied only when the dive is saved.
    let mut edited_samples = use_signal(|| None::<Vec<benthic_core::Sample>>);
    let mut show_profile = use_signal(|| false);
    let mut show_picker = use_signal(|| false);
    let confirm_delete = use_signal(|| false);
    let mut active_dc = use_signal(|| 0usize);

    let dive_id = dive.id;
    let is_editing = (editing)();
    let is_confirming = (confirm_delete)();

    // --- actions ---------------------------------------------------------

    let on_save = {
        let dive = dive.clone();
        let site = site.clone();
        let log = log.clone();
        move |_| {
            let form = form;
            let mut editing = editing;
            let mut edited_samples = edited_samples;
            let f = form();
            let mut after = dive.clone();
            after.when = f.when;
            after.number = f.number;
            after.buddy = f.buddy;
            after.diveguide = f.divemaster;
            after.suit = f.suit;
            after.notes = f.notes;
            after.rating = f.rating;
            after.tags = parse_tags(&f.tags);
            after.trip_id = f.trip_id;
            after.cylinders = f.cylinders;
            after.weights = f.weights;

            // A profile edited in the profile editor replaces the samples;
            // max depth and bottom time are derived from it.
            if let Some(samples) = (edited_samples)() {
                let (max_depth, duration) = profile_bounds(&samples);
                after.max_depth = max_depth;
                after.duration = duration;
                if let Some(computer) = after.computers.first_mut() {
                    computer.samples = samples;
                    computer.max_depth = max_depth;
                    computer.duration = duration;
                    if computer.model.is_empty() {
                        computer.model = "Manually entered".to_string();
                    }
                }
            }

            let mut commands: Vec<Command> = Vec::new();
            let name = f.site_name.trim().to_string();
            let location = parse_location(&f.site_gps);
            let site_id = match (after.site_id, site.clone()) {
                (Some(id), Some(existing)) => {
                    let mut updated = existing.clone();
                    updated.name = name;
                    updated.location = location;
                    commands.push(Command::UpdateSite {
                        before: existing,
                        after: updated,
                    });
                    Some(id)
                }
                _ if !name.is_empty() || location.is_some() => {
                    let uuid = log.next_site_uuid();
                    commands.push(Command::AddSite {
                        site: DiveSite {
                            uuid,
                            name,
                            location,
                            ..Default::default()
                        },
                    });
                    Some(uuid)
                }
                _ => None,
            };
            after.site_id = site_id;
            commands.push(Command::UpdateDive {
                before: dive.clone(),
                after,
            });
            state.dispatch_all("Edit dive", commands);
            editing.set(false);
            edited_samples.set(None);
            state.set_status(tr.saved_dive);
        }
    };

    let on_cancel = {
        let dive = dive.clone();
        let site = site.clone();
        move |_| {
            let mut form = form;
            let mut editing = editing;
            let mut edited_samples = edited_samples;
            form.set(DiveForm::from_dive(&dive, site.as_ref()));
            editing.set(false);
            edited_samples.set(None);
        }
    };

    let on_delete = move |_| {
        let mut confirm = confirm_delete;
        if !confirm() {
            confirm.set(true);
            return;
        }
        actions::delete_dive(state, dive_id);
    };

    let on_duplicate = move |_| actions::duplicate_dive(state, dive_id);

    // --- read-only values -------------------------------------------------

    let title = crate::format::dive_title(&dive, &log);
    let site_name = site
        .as_ref()
        .map(|s| s.name.clone())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "—".to_string());
    let trip_name = dive
        .trip_id
        .and_then(|id| log.trip_by_id(id))
        .map(|t| {
            if t.location.is_empty() {
                format!("Trip #{}", t.id)
            } else {
                t.location.clone()
            }
        })
        .unwrap_or_else(|| "—".to_string());
    let when = prefs.timestamp(dive.when);
    let duration = dive
        .duration()
        .map(format_duration)
        .unwrap_or_else(|| "—".to_string());
    let max_depth = dive
        .max_depth()
        .map(|d| prefs.depth(d))
        .unwrap_or_else(|| "—".to_string());
    let water_temp = dive
        .water_temp
        .or_else(|| dive.primary_computer().and_then(|dc| dc.water_temp))
        .map(|t| prefs.temperature(t))
        .unwrap_or_else(|| "—".to_string());
    let air_temp = dive
        .air_temp
        .or_else(|| dive.primary_computer().and_then(|dc| dc.air_temp))
        .map(|t| prefs.temperature(t))
        .unwrap_or_else(|| "—".to_string());
    let computer = {
        let index = (active_dc)();
        let model = dive
            .computers
            .get(index)
            .map(|dc| dc.model.clone())
            .filter(|m| !m.is_empty())
            .unwrap_or_else(|| "—".to_string());
        if dive.computers.len() > 1 {
            format!("{model} ({}/{})", index + 1, dive.computers.len())
        } else {
            model
        }
    };
    let salinity = salinity_label(
        dive.salinity
            .unwrap_or_else(|| prefs.default_salinity.value()),
    );
    let salinity_value = dive
        .salinity
        .unwrap_or_else(|| prefs.default_salinity.value());
    let avg_depth = dive
        .average_depth()
        .map(|d| prefs.depth(d))
        .unwrap_or_else(|| "—".to_string());
    let rmv = dive
        .rmv_l_per_min()
        .map(|v| format!("{v:.1} L/min"))
        .unwrap_or_else(|| "—".to_string());

    // --- edit-form values -------------------------------------------------

    let f = (form)();
    let when_value = benthic_core::datetime_local(f.when);
    // The profile can be edited by hand only when it is not a dive-computer
    // recording; imported traces are left untouched.
    let profile_editable = dive
        .primary_computer()
        .map(|dc| dc.samples.is_empty() || dc.model == "Manually entered")
        .unwrap_or(true);
    let profile_samples: Option<Vec<benthic_core::Sample>> = (edited_samples)();
    let (profile_max, profile_time) = match &profile_samples {
        Some(samples) => profile_bounds(samples),
        None => (
            dive.max_depth()
                .or_else(|| dive.primary_computer().and_then(|dc| dc.max_depth)),
            dive.duration().or_else(|| {
                dive.primary_computer()
                    .and_then(|dc| dc.duration_or_last_sample())
            }),
        ),
    };
    let profile_summary = i18n::t2(
        tr.profile_summary,
        profile_max
            .map(|depth| prefs.depth(depth))
            .unwrap_or_else(|| "—".to_string()),
        profile_time
            .map(format_duration)
            .unwrap_or_else(|| "—".to_string()),
    );
    // Preview the edited profile in the chart before the form is saved.
    let preview_dive = match &profile_samples {
        Some(samples) => Dive {
            max_depth: profile_max,
            duration: profile_time,
            computers: vec![benthic_core::DiveComputer {
                model: dive
                    .primary_computer()
                    .map(|dc| dc.model.clone())
                    .unwrap_or_default(),
                samples: samples.clone(),
                max_depth: profile_max,
                duration: profile_time,
                ..Default::default()
            }],
            ..dive.clone()
        },
        None => dive.clone(),
    };
    let preview_dc = if profile_samples.is_some() {
        0
    } else {
        (active_dc)()
    };
    let buddy_options = unique_values(log.dives.iter().map(|d| d.buddy.as_str()));
    let divemaster_options = unique_values(log.dives.iter().map(|d| d.diveguide.as_str()));
    let suit_options = unique_values(log.dives.iter().map(|d| d.suit.as_str()));
    let tag_options: Vec<String> = {
        let mut tags: Vec<String> = log
            .dives
            .iter()
            .flat_map(|d| d.tags.iter().cloned())
            .filter(|t| !t.trim().is_empty())
            .collect();
        tags.sort();
        tags.dedup();
        tags
    };
    // The most recent earlier dive, for the one-tap copy shortcut.
    let previous_dive = log
        .dives
        .iter()
        .filter(|d| d.id != dive.id && d.when <= dive.when)
        .max_by_key(|d| d.when)
        .cloned();

    #[cfg(target_arch = "wasm32")]
    let location_button = Some(rsx! {
        button {
            class: "btn",
            r#type: "button",
            title: "{tr.use_my_location_title}",
            onclick: move |_| {
                let mut form = form;
                spawn(async move {
                    if let Some((lat, lon)) = crate::platform::current_location().await {
                        form.write().site_gps = format!("{lat:.6}, {lon:.6}");
                    }
                });
            },
            "{tr.use_my_location}"
        }
    });
    #[cfg(not(target_arch = "wasm32"))]
    let location_button: Option<Element> = None;

    let stars: Vec<(u8, &'static str)> = (1..=5u8)
        .map(|s| (s, if f.rating >= s { "star active" } else { "star" }))
        .collect();
    let trips: Vec<(u32, String)> = log
        .trips
        .iter()
        .map(|t| {
            let label = if t.location.is_empty() {
                i18n::t1(tr.trip_fallback, t.id)
            } else {
                t.location.clone()
            };
            (t.id, label)
        })
        .collect();
    let current_trip = f.trip_id.map(|id| id.to_string()).unwrap_or_default();

    let cylinders: Vec<CylinderRow> = dive
        .cylinders
        .iter()
        .map(|c| cylinder_row(c, &prefs, salinity_value))
        .collect();
    let weights: Vec<WeightRow> = dive.weights.iter().map(|w| weight_row(w, &prefs)).collect();

    rsx! {
        section { class: "detail",
            header { class: "detail-head",
                div { class: "detail-title-row",
                    button {
                        class: "btn back-btn",
                        title: "{tr.back_to_list}",
                        onclick: move |_| mobile_detail.set(false),
                        "‹ {tr.cat_dives}"
                    }
                    h1 { "{title}" }
                    div { class: "detail-actions",
                        if is_editing {
                            button { class: "btn primary", onclick: on_save, "{tr.save}" }
                            button { class: "btn", onclick: on_cancel, "{tr.cancel}" }
                        } else {
                            button { class: "btn", onclick: move |_| editing.set(true), "{tr.edit}" }
                            button { class: "btn", onclick: on_duplicate, "{tr.duplicate}" }
                            button {
                                class: if is_confirming { "btn danger" } else { "btn" },
                                onclick: on_delete,
                                if is_confirming { "{tr.confirm_delete}" } else { "{tr.delete}" }
                            }
                        }
                    }
                }
                p { class: "muted", "{when} · {site_name} · {trip_name}" }
            }

            if dive.computers.len() > 1 {
                div { class: "dc-tabs",
                    for (index, computer) in dive.computers.iter().enumerate() {
                        button {
                            key: "{index}",
                            class: if (active_dc)() == index { "dc-tab active" } else { "dc-tab" },
                            onclick: move |_| active_dc.set(index),
                            if computer.model.is_empty() {
                                {i18n::t1(tr.computer_n, index + 1)}
                            } else {
                                "{computer.model}"
                            }
                        }
                    }
                }
            }

            DiveProfile { dive: preview_dive, dc_index: preview_dc }

            if is_editing {
                div { class: "edit-form",
                    // Native autocomplete for the fields divers retype every
                    // trip. The datalists are invisible; they feed the inputs
                    // below on desktop and mobile alike.
                    datalist { id: "buddy-list",
                        for value in buddy_options.clone() {
                            option { value: "{value}" }
                        }
                    }
                    datalist { id: "divemaster-list",
                        for value in divemaster_options.clone() {
                            option { value: "{value}" }
                        }
                    }
                    datalist { id: "suit-list",
                        for value in suit_options.clone() {
                            option { value: "{value}" }
                        }
                    }
                    datalist { id: "tag-list",
                        for value in tag_options.clone() {
                            option { value: "{value}" }
                        }
                    }
                    if let Some(previous) = previous_dive.clone() {
                        div { class: "field-label full",
                            button {
                                class: "btn",
                                r#type: "button",
                                onclick: move |_| {
                                    let mut w = form.write();
                                    if !previous.buddy.is_empty() {
                                        w.buddy.clone_from(&previous.buddy);
                                    }
                                    if !previous.diveguide.is_empty() {
                                        w.divemaster.clone_from(&previous.diveguide);
                                    }
                                    if !previous.suit.is_empty() {
                                        w.suit.clone_from(&previous.suit);
                                    }
                                },
                                "{tr.copy_previous}"
                            }
                        }
                    }
                    label { class: "field-label", "{tr.field_date_time}"
                        input {
                            class: "field",
                            r#type: "datetime-local",
                            value: "{when_value}",
                            oninput: move |evt| {
                                if let Some(when) = benthic_core::parse_datetime_local(&evt.value()) {
                                    form.write().when = when;
                                }
                            },
                        }
                    }
                    div { class: "field-label full",
                        span { class: "label-row", "{tr.profile}" }
                        div { class: "muted", "{profile_summary}" }
                        div { class: "detail-actions",
                            if profile_editable {
                                button {
                                    class: "btn",
                                    r#type: "button",
                                    onclick: move |_| show_profile.set(true),
                                    "{tr.edit_profile}"
                                }
                            } else {
                                span { class: "muted", "{tr.profile_from_computer}" }
                            }
                        }
                    }
                    label { class: "field-label", "{tr.field_number}"
                        input {
                            class: "field",
                            r#type: "number",
                            value: "{f.number}",
                            oninput: move |evt| form.write().number = evt.value().parse().unwrap_or(0),
                        }
                    }
                    label { class: "field-label", "{tr.field_rating}"
                        div { class: "stars",
                            for (star, class) in stars {
                                button {
                                    key: "{star}",
                                    class: "{class}",
                                    aria_label: i18n::t1(tr.rate_stars, star),
                                    onclick: move |_| form.write().rating = star,
                                    "★"
                                }
                            }
                        }
                    }
                    label { class: "field-label", "{tr.field_buddy}"
                        input {
                            class: "field",
                            list: "buddy-list",
                            value: "{f.buddy}",
                            oninput: move |evt| form.write().buddy = evt.value(),
                        }
                    }
                    label { class: "field-label", "{tr.field_dive_master}"
                        input {
                            class: "field",
                            list: "divemaster-list",
                            value: "{f.divemaster}",
                            oninput: move |evt| form.write().divemaster = evt.value(),
                        }
                    }
                    label { class: "field-label", "{tr.field_suit}"
                        input {
                            class: "field",
                            list: "suit-list",
                            value: "{f.suit}",
                            oninput: move |evt| form.write().suit = evt.value(),
                        }
                    }
                    label { class: "field-label", "{tr.field_trip}"
                        select {
                            class: "field",
                            value: "{current_trip}",
                            onchange: move |evt| {
                                let value = evt.value();
                                form.write().trip_id = if value.is_empty() { None } else { value.parse().ok() };
                            },
                            option { value: "", selected: f.trip_id.is_none(), "{tr.no_trip}" }
                            for (id, label) in trips {
                                option {
                                    key: "{id}",
                                    value: "{id}",
                                    selected: f.trip_id == Some(id),
                                    "{label}"
                                }
                            }
                        }
                    }
                    label { class: "field-label", "{tr.field_site_name}"
                        input {
                            class: "field",
                            value: "{f.site_name}",
                            oninput: move |evt| form.write().site_name = evt.value(),
                        }
                    }
                    label { class: "field-label", "{tr.field_site_gps}"
                        div { class: "gps-row",
                            input {
                                class: "field",
                                inputmode: "decimal",
                                placeholder: "28.572100, 34.536700",
                                value: "{f.site_gps}",
                                oninput: move |evt| form.write().site_gps = evt.value(),
                            }
                            {location_button}
                            button {
                                class: "btn",
                                r#type: "button",
                                title: "{tr.choose_on_map}",
                                onclick: move |_| show_picker.set(true),
                                "{tr.map}"
                            }
                        }
                    }
                    label { class: "field-label full", "{tr.field_tags}"
                        input {
                            class: "field",
                            list: "tag-list",
                            value: "{f.tags}",
                            oninput: move |evt| form.write().tags = evt.value(),
                        }
                        if !tag_options.is_empty() {
                            div { class: "tag-suggest",
                                {tag_options.iter().take(20).map(|tag| {
                                    let tag = tag.clone();
                                    rsx! {
                                        button {
                                            key: "{tag}",
                                            class: "tag tag-add",
                                            r#type: "button",
                                            onclick: move |_| {
                                                let mut w = form.write();
                                                let mut tags = parse_tags(&w.tags);
                                                if !tags.contains(&tag) {
                                                    tags.push(tag.clone());
                                                    w.tags = tags.join(", ");
                                                }
                                            },
                                            "{tag}"
                                        }
                                    }
                                })}
                            }
                        }
                    }
                    label { class: "field-label full", "{tr.field_notes}"
                        textarea {
                            class: "field",
                            rows: "6",
                            value: "{f.notes}",
                            oninput: move |evt| form.write().notes = evt.value(),
                        }
                    }

                    div { class: "field-label full", "{tr.cyl_cylinder}"
                        div { class: "equip-list",
                            for (i, cyl) in f.cylinders.iter().enumerate() {
                                div { key: "{i}", class: "equip-row cylinder-row",
                                    select {
                                        class: "field",
                                        value: preset_value(cyl),
                                        onchange: move |evt| {
                                            if let Some(preset) = cylinder_preset(&evt.value()) {
                                                let mut w = form.write();
                                                apply_preset(&mut w.cylinders[i], preset);
                                            }
                                        },
                                        option { value: "", selected: preset_value(cyl).is_empty(), "{tr.custom}" }
                                        for preset in CYLINDER_PRESETS {
                                            option {
                                                key: "{preset.name}",
                                                value: "{preset.name}",
                                                selected: preset_value(cyl) == preset.name,
                                                "{preset.name}"
                                            }
                                        }
                                    }
                                    label { class: "mini", "{tr.cyl_o2}"
                                        input {
                                            class: "field",
                                            value: format!("{:.1}", cyl.gas.o2_percent()),
                                            oninput: move |evt| {
                                                let permille = evt.value().parse::<f64>().map(|v| (v * 10.0).round() as u16).unwrap_or(0);
                                                form.write().cylinders[i].gas.o2_permille = permille;
                                            },
                                        }
                                    }
                                    label { class: "mini", "{tr.cyl_he}"
                                        input {
                                            class: "field",
                                            value: format!("{:.1}", cyl.gas.he_percent()),
                                            oninput: move |evt| {
                                                let permille = evt.value().parse::<f64>().map(|v| (v * 10.0).round() as u16).unwrap_or(0);
                                                form.write().cylinders[i].gas.he_permille = permille;
                                            },
                                        }
                                    }
                                    label { class: "mini", "{tr.cyl_start} {prefs.pressure_unit()}"
                                        input {
                                            class: "field",
                                            value: cyl.start_pressure.map(|p| format!("{:.0}", prefs.pressure_value(p))).unwrap_or_default(),
                                            oninput: move |evt| {
                                                form.write().cylinders[i].start_pressure = evt.value().parse::<f64>().ok().map(|v| prefs.pressure_from_value(v));
                                            },
                                        }
                                    }
                                    label { class: "mini", "{tr.cyl_end} {prefs.pressure_unit()}"
                                        input {
                                            class: "field",
                                            value: cyl.end_pressure.map(|p| format!("{:.0}", prefs.pressure_value(p))).unwrap_or_default(),
                                            oninput: move |evt| {
                                                form.write().cylinders[i].end_pressure = evt.value().parse::<f64>().ok().map(|v| prefs.pressure_from_value(v));
                                            },
                                        }
                                    }
                                    label { class: "mini", "{tr.cyl_use}"
                                        select {
                                            class: "field",
                                            value: "{cyl.use_.index()}",
                                            onchange: move |evt| {
                                                let index = evt.value().parse::<usize>().unwrap_or(0);
                                                form.write().cylinders[i].use_ = CylinderUse::from_index(index);
                                            },
                                            for use_ in CylinderUse::ALL {
                                                option {
                                                    key: "{use_.index()}",
                                                    value: "{use_.index()}",
                                                    selected: cyl.use_ == use_,
                                                    "{use_.label()}"
                                                }
                                            }
                                        }
                                    }
                                    button {
                                        class: "icon-btn",
                                        title: "{tr.cyl_remove}",
                                        onclick: move |_| { form.write().cylinders.remove(i); },
                                        "✕"
                                    }
                                }
                            }
                        }
                        button {
                            class: "btn",
                            onclick: move |_| {
                                let cylinder = match prefs
                                    .default_cylinder
                                    .and_then(|index| CYLINDER_PRESETS.get(index))
                                {
                                    Some(preset) => Cylinder::from_preset(preset),
                                    None => Cylinder::default(),
                                };
                                form.write().cylinders.push(cylinder);
                            },
                            "{tr.cyl_add}"
                        }
                    }

                    div { class: "field-label full", "{tr.weight_weight}"
                        div { class: "equip-list",
                            for (i, ws) in f.weights.iter().enumerate() {
                                div { key: "{i}", class: "equip-row weight-row",
                                    label { class: "mini", "{prefs.weight_unit()}"
                                        input {
                                            class: "field",
                                            value: format!("{:.2}", prefs.weight_value(ws.weight)),
                                            oninput: move |evt| {
                                                form.write().weights[i].weight = prefs.weight_from_value(evt.value().parse::<f64>().unwrap_or(0.0));
                                            },
                                        }
                                    }
                                    label { class: "mini wide", "{tr.weight_description}"
                                        input {
                                            class: "field",
                                            value: "{ws.description}",
                                            oninput: move |evt| form.write().weights[i].description = evt.value(),
                                        }
                                    }
                                    button {
                                        class: "icon-btn",
                                        title: "{tr.weight_remove}",
                                        onclick: move |_| { form.write().weights.remove(i); },
                                        "✕"
                                    }
                                }
                            }
                        }
                        button {
                            class: "btn",
                            onclick: move |_| form.write().weights.push(WeightSystem::new(Weight::from_kg(0.0), "belt")),
                            "{tr.weight_add}"
                        }
                    }
                }
            } else {
                div { class: "facts",
                    Fact { label: tr.fact_duration, value: duration }
                    Fact { label: tr.fact_max_depth, value: max_depth }
                    Fact { label: tr.fact_avg_depth, value: avg_depth }
                    Fact { label: tr.fact_rmv, value: rmv }
                    Fact { label: tr.fact_water_temp, value: water_temp }
                    Fact { label: tr.fact_air_temp, value: air_temp }
                    Fact { label: tr.fact_computer, value: computer }
                    Fact { label: tr.fact_buddy, value: dive.buddy.clone() }
                    Fact { label: tr.fact_dive_master, value: dive.diveguide.clone() }
                    Fact { label: tr.fact_suit, value: dive.suit.clone() }
                    Fact { label: tr.fact_salinity, value: salinity }
                }

                if let Some(location) = site.as_ref().and_then(|s| s.location).filter(|l| l.is_valid()) {
                    div { class: "detail-map-wrap",
                        MapView {
                            sites: vec![MapSite {
                                site_id: site.as_ref().map(|s| s.uuid).unwrap_or_default(),
                                name: site
                                    .as_ref()
                                    .map(|s| s.name.clone())
                                    .unwrap_or_default(),
                                lat: location.lat,
                                lon: location.lon,
                                dive_id: Some(dive.id),
                                dives: 1,
                                country: site.as_ref().and_then(|s| s.country.clone()),
                            }],
                            max_zoom: 13,
                            height: "240px".to_string(),
                        }
                        a {
                            class: "map-link",
                            target: "_blank",
                            rel: "noopener",
                            href: "https://www.openstreetmap.org/?mlat={location.lat}&mlon={location.lon}#map=15/{location.lat}/{location.lon}",
                            "{tr.open_osm}"
                        }
                    }
                }

                if !dive.tags.is_empty() {
                    div { class: "tags",
                        for tag in dive.tags.clone() {
                            span { key: "{tag}", class: "tag", "{tag}" }
                        }
                    }
                }

                if !cylinders.is_empty() {
                    div { class: "section-title", "{tr.section_gas}" }
                    div { class: "table-scroll",
                        table { class: "data-table",
                            thead { tr { th { "{tr.cyl_cylinder}" } th { "{tr.cyl_gas}" } th { "{tr.cyl_mod}" } th { "{tr.cyl_start}" } th { "{tr.cyl_end}" } } }
                            tbody {
                                for row in cylinders {
                                    tr {
                                        td { "{row.description}" }
                                        td { "{row.gas}" }
                                        td { "{row.mod_depth}" }
                                        td { "{row.start}" }
                                        td { "{row.end}" }
                                    }
                                }
                            }
                        }
                    }
                    if !weights.is_empty() {
                        div { class: "table-scroll",
                            table { class: "data-table",
                                thead { tr { th { "{tr.weight_weight}" } th { "{tr.weight_description}" } } }
                                tbody {
                                    for row in weights {
                                        tr { td { "{row.weight}" } td { "{row.description}" } }
                                    }
                                }
                            }
                        }
                    }
                }

                if !dive.notes.is_empty() {
                    div { class: "section-title", "{tr.section_notes}" }
                    p { class: "notes", "{dive.notes}" }
                }
            }

            if (show_picker)() {
                LocationPickerDialog {
                    on_choose: move |(lat, lon)| {
                        form.write().site_gps = format!("{lat:.6}, {lon:.6}");
                        show_picker.set(false);
                    },
                    on_close: move |_| show_picker.set(false),
                }
            }

            if (show_profile)() {
                ProfileEditorDialog {
                    initial: (edited_samples)()
                        .or_else(|| dive.primary_computer().map(|dc| dc.samples.clone()))
                        .unwrap_or_default(),
                    on_save: move |samples| {
                        edited_samples.set(Some(samples));
                        show_profile.set(false);
                    },
                    on_close: move |_| show_profile.set(false),
                }
            }
        }
    }
}

#[component]
fn Fact(label: &'static str, value: String) -> Element {
    rsx! {
        div { class: "fact",
            span { class: "fact-label", "{label}" }
            span { class: "fact-value", "{value}" }
        }
    }
}

struct CylinderRow {
    description: String,
    gas: String,
    mod_depth: String,
    start: String,
    end: String,
}

struct WeightRow {
    description: String,
    weight: String,
}

fn cylinder_row(
    cyl: &benthic_core::Cylinder,
    prefs: &benthic_core::Preferences,
    salinity: i32,
) -> CylinderRow {
    let mod_mm = benthic_core::gas::mod_depth_mm(
        cyl.gas,
        benthic_core::gas::DEFAULT_PO2_LIMIT_MBAR,
        benthic_core::gas::SURFACE_PRESSURE_MBAR,
        salinity,
    );
    CylinderRow {
        description: if cyl.description.is_empty() {
            cyl.size
                .map(|s| prefs.volume(s))
                .unwrap_or_else(|| "—".to_string())
        } else {
            cyl.description.clone()
        },
        gas: cyl.gas.name(),
        mod_depth: prefs.depth(benthic_core::Depth::new(mod_mm)),
        start: cyl
            .start_pressure
            .map(|p| prefs.pressure(p))
            .unwrap_or_else(|| "—".to_string()),
        end: cyl
            .end_pressure
            .map(|p| prefs.pressure(p))
            .unwrap_or_else(|| "—".to_string()),
    }
}

fn weight_row(ws: &benthic_core::WeightSystem, prefs: &benthic_core::Preferences) -> WeightRow {
    WeightRow {
        description: if ws.description.is_empty() {
            "—".to_string()
        } else {
            ws.description.clone()
        },
        weight: prefs.weight(ws.weight),
    }
}

/// A human label for a salinity value, e.g. 10200 -> "EN13319".
fn salinity_label(value: i32) -> String {
    benthic_core::Salinity::ALL
        .iter()
        .find(|s| s.value() == value)
        .map(|s| s.label().to_string())
        .unwrap_or_else(|| format!("{value} g/10L"))
}

/// The matching preset name for a cylinder, or empty when it is custom.
fn preset_value(cylinder: &Cylinder) -> String {
    if is_preset(&cylinder.description) {
        cylinder.description.clone()
    } else {
        String::new()
    }
}

/// Parse a comma-separated tag list into a sorted, de-duplicated vector.
fn parse_tags(text: &str) -> Vec<String> {
    let mut tags: Vec<String> = text
        .split(',')
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();
    tags.sort();
    tags.dedup();
    tags
}

/// Distinct, non-empty values in sorted order, for autocomplete lists.
fn unique_values<'a>(values: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut out: Vec<String> = values
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Parse "lat, lon" or "lat lon" into a valid [`Location`].
fn parse_location(text: &str) -> Option<Location> {
    let cleaned = text.replace(',', " ");
    let mut parts = cleaned.split_whitespace();
    let lat: f64 = parts.next()?.parse().ok()?;
    let lon: f64 = parts.next()?.parse().ok()?;
    let location = Location::new(lat, lon);
    location.is_valid().then_some(location)
}
