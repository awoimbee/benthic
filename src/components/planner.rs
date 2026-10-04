use dioxus::prelude::*;

use benthic_core::deco::{BreathingMode, PlanPoint, PscrParams};
use benthic_core::gas::{
    ambient_mbar, end_depth_mm, mod_depth_mm, GasMix, DEFAULT_PO2_LIMIT_MBAR, SURFACE_PRESSURE_MBAR,
};
use benthic_core::planner::ndl as plan_ndl;
use benthic_core::units::{format_duration, Depth, Duration};
use benthic_core::{DecoModel, Dive, DiveComputer, DivePlan, Preferences};

use crate::actions;
use crate::components::{DiveProfile, InfoTip};
use crate::i18n;
use crate::state::AppState;

/// One editable waypoint row: a segment ending at `depth` after `minutes`.
#[derive(Clone, PartialEq)]
struct PointForm {
    depth: f64,
    minutes: f64,
    o2: f64,
    he: f64,
    mode_index: usize,
    setpoint: f64,
    dump_ratio: f64,
}

impl PointForm {
    fn breathing_mode(&self) -> BreathingMode {
        let diluent = GasMix::percent(self.o2, self.he);
        match self.mode_index {
            1 => BreathingMode::ClosedCircuit {
                diluent,
                setpoint_bar: self.setpoint,
            },
            2 => BreathingMode::PassiveSemiClosed {
                diluent,
                params: PscrParams {
                    dump_ratio: self.dump_ratio.max(1.0),
                    ..Default::default()
                },
            },
            _ => BreathingMode::OpenCircuit(diluent),
        }
    }

    fn gas(&self) -> GasMix {
        GasMix::percent(self.o2, self.he)
    }

    fn to_plan_point(&self, prefs: &Preferences) -> PlanPoint {
        PlanPoint {
            depth: prefs.depth_from_value(self.depth),
            duration: Duration::from_minutes(self.minutes.round().max(0.0) as i32),
            mode: self.breathing_mode(),
        }
    }
}

impl Default for PointForm {
    fn default() -> Self {
        Self {
            depth: 30.0,
            minutes: 20.0,
            o2: 21.0,
            he: 0.0,
            mode_index: 0,
            setpoint: 1.3,
            dump_ratio: 100.0,
        }
    }
}

/// The inputs that determine the NDL, so it is only recomputed when one of them
/// changes (the VPM-B NDL is a binary search over full schedules).
#[derive(Clone, Copy, PartialEq)]
struct NdlKey {
    depth_mm: i32,
    o2: u16,
    he: u16,
    mode_index: usize,
    setpoint_100: i32,
    dump_ratio: i32,
    gf_low_100: i32,
    gf_high_100: i32,
    model_index: usize,
    conservatism: u8,
    salinity: i32,
}

/// A dive planner: build a multi-level profile from waypoints and see the
/// generated decompression schedule, gas needs and the no-decompression limit.
#[component]
pub fn PlannerDialog() -> Element {
    let state = use_context::<AppState>();
    let mut show_planner = state.show_planner;
    let prefs = (state.prefs)();
    let tr = i18n::strings(prefs.language);
    let salinity = prefs.default_salinity.value();

    let mut forms = use_signal(|| {
        vec![
            PointForm {
                depth: 30.0,
                minutes: 1.5,
                ..Default::default()
            },
            PointForm::default(),
        ]
    });
    let mut gf_low = use_signal(|| 0.30f64);
    let mut gf_high = use_signal(|| 0.70f64);
    let mut deco_model_index = use_signal(|| 0usize);
    let mut vpmb_conservatism = use_signal(|| 3u8);
    let mut rmv = use_signal(|| 20.0f64);
    let mut ndl = use_signal(|| None::<Duration>);
    let mut ndl_key = use_signal(|| None::<NdlKey>);
    // Which waypoint cards are open. Only the descent starts open, so a phone
    // shows the whole plan at a glance instead of a wall of fields.
    let mut expanded = use_signal(|| {
        let mut set = std::collections::HashSet::new();
        set.insert(0usize);
        set
    });

    // Recompute the NDL only when its inputs change, not on every edit.
    use_effect(move || {
        let form_list = (forms)();
        let prefs = (state.prefs)();
        let salinity = prefs.default_salinity.value();
        let model_index = (deco_model_index)();
        let conservatism = (vpmb_conservatism)();
        let gf_l = (gf_low)();
        let gf_h = (gf_high)();
        let Some(last) = form_list.last() else {
            ndl.set(None);
            return;
        };
        let gas = last.gas();
        let key = NdlKey {
            depth_mm: prefs.depth_from_value(last.depth).mm,
            o2: gas.o2_permille,
            he: gas.he_permille,
            mode_index: last.mode_index,
            setpoint_100: (last.setpoint * 100.0).round() as i32,
            dump_ratio: last.dump_ratio.round() as i32,
            gf_low_100: (gf_l * 100.0).round() as i32,
            gf_high_100: (gf_h * 100.0).round() as i32,
            model_index,
            conservatism,
            salinity,
        };
        if *ndl_key.peek() == Some(key) {
            return;
        }
        ndl_key.set(Some(key));
        let deco_model = if model_index == 1 {
            DecoModel::Vpmb { conservatism }
        } else {
            DecoModel::Buhlmann {
                gf_low: gf_l,
                gf_high: gf_h,
            }
        };
        ndl.set(plan_ndl(
            prefs.depth_from_value(last.depth),
            last.breathing_mode(),
            deco_model,
            SURFACE_PRESSURE_MBAR / 1000.0,
            salinity,
        ));
    });

    let form_list = (forms)();
    let gf_low_value = (gf_low)();
    let gf_high_value = (gf_high)();
    let deco_model_value = (deco_model_index)();
    let vpmb_conservatism_value = (vpmb_conservatism)();
    let deco_model = if deco_model_value == 1 {
        DecoModel::Vpmb {
            conservatism: vpmb_conservatism_value,
        }
    } else {
        DecoModel::Buhlmann {
            gf_low: gf_low_value,
            gf_high: gf_high_value,
        }
    };

    let plan_points: Vec<PlanPoint> = form_list.iter().map(|f| f.to_plan_point(&prefs)).collect();
    let plan = DivePlan::compute(
        plan_points,
        deco_model,
        SURFACE_PRESSURE_MBAR / 1000.0,
        salinity,
    );

    // Per-waypoint run time and cumulative gas used, for the table.
    let mut run_seconds = Vec::with_capacity(form_list.len());
    let mut used_liters = Vec::with_capacity(form_list.len());
    let mut elapsed = 0.0f64;
    let mut used = 0.0f64;
    let mut previous_mm = 0i32;
    for form in &form_list {
        let depth_mm = prefs.depth_from_value(form.depth).mm;
        let average_mm = (previous_mm + depth_mm) / 2;
        let ata = ambient_mbar(average_mm, SURFACE_PRESSURE_MBAR, salinity) / SURFACE_PRESSURE_MBAR;
        used += (rmv)() * form.minutes * ata;
        elapsed += form.minutes;
        run_seconds.push(elapsed);
        used_liters.push(used);
        previous_mm = depth_mm;
    }

    let max_depth = plan.max_depth();
    let bottom_mode = plan.bottom_mode();
    let bottom_gas = plan.bottom_gas();
    let rebreather = bottom_mode.is_rebreather();
    let mod_mm = mod_depth_mm(
        bottom_gas,
        DEFAULT_PO2_LIMIT_MBAR,
        SURFACE_PRESSURE_MBAR,
        salinity,
    );
    let end_mm = end_depth_mm(
        bottom_gas,
        max_depth.mm,
        SURFACE_PRESSURE_MBAR,
        salinity,
        false,
    );
    let over_mod = max_depth.mm > mod_mm;
    // Surface bad gas/depth/time entries instead of silently computing with
    // them. These mirror the bounds a diver would expect.
    let plan_warnings: Vec<String> = form_list
        .iter()
        .enumerate()
        .flat_map(|(index, form)| {
            let waypoint = index + 1;
            let mut issues = Vec::new();
            if form.depth < 0.0 {
                issues.push(i18n::t1(tr.warn_neg_depth, waypoint));
            }
            if form.minutes < 0.0 {
                issues.push(i18n::t1(tr.warn_neg_duration, waypoint));
            }
            if !(1.0..=100.0).contains(&form.o2) {
                issues.push(i18n::t1(tr.warn_o2_range, waypoint));
            }
            if !(0.0..=100.0).contains(&form.he) {
                issues.push(i18n::t1(tr.warn_he_range, waypoint));
            }
            if form.o2 + form.he > 100.0 {
                issues.push(i18n::t1(tr.warn_o2_he_sum, waypoint));
            }
            issues
        })
        .collect();
    let gas_label: &'static str = if rebreather {
        tr.gas_diluent
    } else {
        tr.gas_gas
    };
    let ambient = ambient_mbar(max_depth.mm, SURFACE_PRESSURE_MBAR, salinity) / 1000.0;
    let gas_needs = plan.gas_needs_liters((rmv)(), SURFACE_PRESSURE_MBAR / 1000.0, salinity);
    let gas_bar_12l = gas_needs / 12.0;
    let bailout = plan.bailout_liters((rmv)(), SURFACE_PRESSURE_MBAR / 1000.0, salinity);
    let bailout_bar_12l = bailout / 12.0;

    let ndl_text = ndl()
        .map(|d| {
            if d.seconds >= 24 * 3600 {
                "> 24 h".to_string()
            } else {
                format_duration(d)
            }
        })
        .unwrap_or_else(|| "> 24 h".to_string());

    let stops: Vec<(String, String)> = plan
        .stops
        .iter()
        .map(|stop| (prefs.depth(stop.depth), format_duration(stop.duration)))
        .collect();
    let has_stops = !stops.is_empty();

    let graph_dive = Dive {
        max_depth: Some(max_depth),
        duration: Some(plan.total_time()),
        computers: vec![DiveComputer {
            model: "Planner".to_string(),
            samples: plan.samples(),
            ..Default::default()
        }],
        ..Default::default()
    };

    let deco_label = match deco_model {
        DecoModel::Buhlmann { gf_low, gf_high } => {
            format!("GF {:.0}/{:.0}", gf_low * 100.0, gf_high * 100.0)
        }
        DecoModel::Vpmb { conservatism } => format!("VPM-B +{conservatism}"),
    };
    let header_summary = format!(
        "max {} \u{b7} {} \u{b7} {deco_label}",
        prefs.depth(max_depth),
        format_duration(plan.total_time()),
    );

    let plan_for_save = plan.clone();
    let on_save = move |_| {
        actions::save_plan(state, plan_for_save.clone());
        show_planner.set(false);
    };

    rsx! {
        section { class: "planner-screen",
            role: "dialog",
            tabindex: "-1",
            autofocus: true,
            aria_modal: "true",
            aria_label: "{tr.dive_planner}",
            div { class: "planner-inner",
                header { class: "detail-head",
                    div { class: "detail-title-row",
                        button {
                            class: "btn back-btn",
                            title: "{tr.back}",
                            onclick: move |_| show_planner.set(false),
                            "\u{2039} {tr.back}"
                        }
                        h1 { "{tr.dive_planner}" }
                        div { class: "detail-actions",
                            button { class: "btn primary", onclick: on_save, "{tr.planner_save_dive}" }
                            button {
                                class: "btn",
                                onclick: move |_| show_planner.set(false),
                                "{tr.close}"
                            }
                        }
                    }
                    p { class: "muted", "{header_summary}" }
                }

                DiveProfile { dive: graph_dive, dc_index: 0 }

                div { class: "section-title", "{tr.profile}" }
                div { class: "planner-presets",
                    for (label, depth_value, minutes) in recreational_presets(&prefs) {
                        button {
                            key: "{label}",
                            class: "btn",
                            onclick: move |_| {
                                forms.set(recreational_preset(depth_value, minutes));
                                let mut set = expanded.write();
                                set.clear();
                                set.insert(0);
                            },
                            "{label}"
                        }
                    }
                    button {
                        class: "btn",
                        onclick: move |_| {
                            forms.set(default_forms());
                            let mut set = expanded.write();
                            set.clear();
                            set.insert(0);
                        },
                        "{tr.reset}"
                    }
                }
                div { class: "plan-points",
                    {form_list.iter().enumerate().map(|(index, form)| {
                        let is_open = (expanded)().contains(&index);
                        let form = form.clone();
                        let run = format_duration(Duration::new((run_seconds[index] * 60.0).round() as i32));
                        let used = used_liters[index];
                        rsx! {
                            div { key: "{index}", class: "plan-point",
                                div { class: "plan-point-head",
                                    button {
                                        class: "plan-point-toggle",
                                        r#type: "button",
                                        onclick: move |_| {
                                            let mut set = expanded.write();
                                            if !set.remove(&index) {
                                                set.insert(index);
                                            }
                                        },
                                        span { class: "plan-point-caret", if is_open { "\u{25be}" } else { "\u{25b8}" } }
                                        span { class: "plan-point-index", "{index + 1}" }
                                        span { class: "plan-point-summary",
                                            "{form.depth:.0} {prefs.depth_unit()} \u{b7} {form.minutes:.0} min \u{b7} {run}"
                                        }
                                        span { class: "plan-point-mode", "{mode_label(form.mode_index)}" }
                                    }
                                    button {
                                        class: "icon-btn point-remove",
                                        title: "{tr.remove_waypoint}",
                                        disabled: form_list.len() <= 1,
                                        onclick: move |_| { forms.write().remove(index); },
                                        "\u{00d7}"
                                    }
                                }
                                if is_open {
                                    div { class: "plan-point-body",
                                        label { class: "field-label", {i18n::t1(tr.field_depth, prefs.depth_unit())}
                                            input {
                                                class: "field",
                                                r#type: "number",
                                                inputmode: "decimal",
                                                value: "{form.depth}",
                                                oninput: move |evt| forms.write()[index].depth = evt.value().parse().unwrap_or(0.0),
                                            }
                                        }
                                        label { class: "field-label", "{tr.duration_min}"
                                            input {
                                                class: "field",
                                                r#type: "number",
                                                inputmode: "numeric",
                                                value: "{form.minutes}",
                                                oninput: move |evt| forms.write()[index].minutes = evt.value().parse().unwrap_or(0.0),
                                            }
                                        }
                                        label { class: "field-label", "{tr.run_time}"
                                            div { class: "readout", "{run}" }
                                        }
                                        label { class: "field-label", "O2 %"
                                            input {
                                                class: "field",
                                                r#type: "number",
                                                inputmode: "decimal",
                                                value: "{form.o2}",
                                                oninput: move |evt| forms.write()[index].o2 = evt.value().parse().unwrap_or(21.0),
                                            }
                                        }
                                        label { class: "field-label", "He %"
                                            input {
                                                class: "field",
                                                r#type: "number",
                                                inputmode: "decimal",
                                                value: "{form.he}",
                                                oninput: move |evt| forms.write()[index].he = evt.value().parse().unwrap_or(0.0),
                                            }
                                        }
                                        label { class: "field-label",
                                            span { class: "label-row",
                                                "{tr.dive_mode}"
                                                InfoTip { text: tr.tip_dive_mode }
                                            }
                                            select {
                                                class: "field",
                                                value: "{form.mode_index}",
                                                onchange: move |evt| forms.write()[index].mode_index = evt.value().parse().unwrap_or(0),
                                                option { value: "0", "{tr.oc_label}" }
                                                option { value: "1", "{tr.ccr_label}" }
                                                option { value: "2", "{tr.pscr_label}" }
                                            }
                                        }
                                        if form.mode_index == 1 {
                                            label { class: "field-label", "{tr.setpoint}"
                                                input {
                                                    class: "field",
                                                    r#type: "number",
                                                    step: "0.1",
                                                    value: "{form.setpoint}",
                                                    oninput: move |evt| forms.write()[index].setpoint = evt.value().parse().unwrap_or(1.3),
                                                }
                                            }
                                        }
                                        if form.mode_index == 2 {
                                            label { class: "field-label", "{tr.dump_ratio}"
                                                input {
                                                    class: "field",
                                                    r#type: "number",
                                                    step: "10",
                                                    value: "{form.dump_ratio}",
                                                    oninput: move |evt| forms.write()[index].dump_ratio = evt.value().parse().unwrap_or(100.0),
                                                }
                                            }
                                        }
                                        label { class: "field-label", "{tr.used_gas}"
                                            div { class: "readout", "{used:.0} L" }
                                        }
                                    }
                                }
                            }
                        }
                    })}
                }
                button {
                    class: "btn",
                    onclick: move |_| {
                        let mut list = forms.write();
                        let next = list
                            .last()
                            .cloned()
                            .map(|mut p| {
                                p.minutes = 0.0;
                                p
                            })
                            .unwrap_or_default();
                        list.push(next);
                    },
                    "{tr.add_waypoint}"
                }

                div { class: "section-title", "{tr.settings}" }
                div { class: "edit-form",
                    label { class: "field-label",
                        span { class: "label-row",
                            "{tr.deco_model}"
                            InfoTip { text: tr.tip_deco_model }
                        }
                        select {
                            class: "field",
                            value: "{deco_model_value}",
                            onchange: move |evt| deco_model_index.set(evt.value().parse().unwrap_or(0)),
                            option { value: "0", "B\u{fc}hlmann (GF)" }
                            option { value: "1", "VPM-B" }
                        }
                    }
                    if deco_model_value == 1 {
                        label { class: "field-label",
                            span { class: "label-row",
                                "{tr.conservatism}"
                                InfoTip { text: tr.tip_conservatism }
                            }
                            select {
                                class: "field",
                                value: "{vpmb_conservatism_value}",
                                onchange: move |evt| vpmb_conservatism.set(evt.value().parse().unwrap_or(3)),
                                for level in 0u8..=4 {
                                    option { value: "{level}", "+{level}" }
                                }
                            }
                        }
                    } else {
                        label { class: "field-label",
                            span { class: "label-row",
                                "{tr.gf_low}"
                                InfoTip { text: tr.tip_gf_low }
                            }
                            input {
                                class: "field",
                                r#type: "number",
                                min: "0.1",
                                max: "1.0",
                                step: "0.05",
                                value: "{gf_low_value}",
                                oninput: move |evt| gf_low.set(evt.value().parse::<f64>().unwrap_or(0.3).clamp(0.1, 1.0)),
                            }
                        }
                        label { class: "field-label",
                            span { class: "label-row",
                                "{tr.gf_high}"
                                InfoTip { text: tr.tip_gf_high }
                            }
                            input {
                                class: "field",
                                r#type: "number",
                                min: "0.1",
                                max: "1.0",
                                step: "0.05",
                                value: "{gf_high_value}",
                                oninput: move |evt| gf_high.set(evt.value().parse::<f64>().unwrap_or(0.7).clamp(0.1, 1.0)),
                            }
                        }
                    }
                    label { class: "field-label",
                        span { class: "label-row",
                            "{tr.rmv}"
                            InfoTip { text: tr.tip_rmv }
                        }
                        input {
                            class: "field",
                            r#type: "number",
                            inputmode: "decimal",
                            value: "{rmv}",
                            oninput: move |evt| rmv.set(evt.value().parse().unwrap_or(20.0)),
                        }
                    }
                }
                if over_mod {
                    p { class: "warn", "{tr.warn_over_mod}" }
                }
                for warning in plan_warnings.iter() {
                    p { class: "warn", "{warning}" }
                }

                div { class: "section-title", "{tr.summary}" }
                div { class: "facts planner-results",
                    Result { label: gas_label, value: bottom_gas.name() }
                    Result { label: tr.ambient, value: format!("{ambient:.2} bar") }
                    Result {
                        label: tr.mod_depth,
                        value: prefs.depth(Depth::new(mod_mm)),
                        help: Some(tr.tip_mod),
                    }
                    Result {
                        label: tr.end,
                        value: prefs.depth(Depth::new(end_mm)),
                        help: Some(tr.tip_end),
                    }
                    Result {
                        label: tr.ndl,
                        value: ndl_text,
                        help: Some(tr.tip_ndl),
                    }
                    Result { label: tr.runtime, value: format_duration(plan.total_time()) }
                    Result { label: tr.bottom_time, value: format_duration(plan.bottom_time()) }
                    Result { label: tr.deco_time, value: format_duration(plan.deco_time()) }
                    if !rebreather {
                        Result {
                            label: tr.gas_needed,
                            value: format!("{gas_needs:.0} L"),
                            help: Some(tr.tip_gas_needed),
                        }
                        Result { label: "\u{2248} 12 L fills", value: format!("{gas_bar_12l:.0} bar") }
                    }
                    Result {
                        label: tr.oc_bailout,
                        value: format!("{bailout:.0} L"),
                        help: Some(tr.tip_bailout),
                    }
                    Result { label: "\u{2248} 12 L bailout", value: format!("{bailout_bar_12l:.0} bar") }
                }

                if has_stops {
                    div { class: "section-title", "{tr.deco_schedule}" }
                    table { class: "data-table",
                        thead { tr { th { "{tr.stop}" } th { "{tr.time}" } } }
                        tbody {
                            for (stop_depth, stop_time) in stops {
                                tr { td { "{stop_depth}" } td { "{stop_time}" } }
                            }
                        }
                    }
                } else {
                    p { class: "muted", "{tr.no_stops}" }
                }
            }
        }
    }
}

#[component]
fn Result(
    label: &'static str,
    value: String,
    #[props(default)] help: Option<&'static str>,
) -> Element {
    rsx! {
        div { class: "fact",
            span { class: "fact-label",
                "{label}"
                if let Some(help) = help {
                    InfoTip { text: help }
                }
            }
            span { class: "fact-value", "{value}" }
        }
    }
}

/// Quick-start waypoint pairs, labelled in the active units: (label, depth
/// value in the active unit, bottom minutes).
fn recreational_presets(prefs: &Preferences) -> Vec<(String, f64, f64)> {
    [(18.0, 40.0), (30.0, 20.0), (40.0, 15.0)]
        .into_iter()
        .map(|(metres, minutes)| {
            let depth = Depth::from_meters(metres);
            (
                format!("{} / {minutes:.0} min", prefs.depth(depth)),
                prefs.depth_value(depth),
                minutes,
            )
        })
        .collect()
}

fn recreational_preset(depth_value: f64, minutes: f64) -> Vec<PointForm> {
    vec![
        PointForm {
            depth: depth_value,
            minutes: 1.5,
            ..Default::default()
        },
        PointForm {
            depth: depth_value,
            minutes,
            ..Default::default()
        },
    ]
}

fn default_forms() -> Vec<PointForm> {
    vec![
        PointForm {
            depth: 30.0,
            minutes: 1.5,
            ..Default::default()
        },
        PointForm::default(),
    ]
}

fn mode_label(index: usize) -> &'static str {
    match index {
        1 => "CCR",
        2 => "pSCR",
        _ => "OC",
    }
}
