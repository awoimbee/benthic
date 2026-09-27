use dioxus::prelude::*;

use benthic_core::deco::{BreathingMode, PlanPoint, PscrParams};
use benthic_core::gas::{
    ambient_mbar, end_depth_mm, mod_depth_mm, GasMix, DEFAULT_PO2_LIMIT_MBAR, SURFACE_PRESSURE_MBAR,
};
use benthic_core::planner::ndl as plan_ndl;
use benthic_core::units::{format_duration, Depth, Duration};
use benthic_core::{DecoModel, Dive, DiveComputer, DivePlan, Preferences};

use crate::actions;
use crate::components::DiveProfile;
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
    let gas_label: &'static str = if rebreather { "Diluent" } else { "Gas" };
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
            div { class: "planner-inner",
                header { class: "detail-head",
                    div { class: "detail-title-row",
                        button {
                            class: "btn back-btn",
                            title: "Back",
                            onclick: move |_| show_planner.set(false),
                            "\u{2039} Back"
                        }
                        h1 { "Dive planner" }
                        div { class: "detail-actions",
                            button { class: "btn primary", onclick: on_save, "Save as dive" }
                            button {
                                class: "btn",
                                onclick: move |_| show_planner.set(false),
                                "Close"
                            }
                        }
                    }
                    p { class: "muted", "{header_summary}" }
                }

                DiveProfile { dive: graph_dive, dc_index: 0 }

                div { class: "section-title", "Profile" }
                div { class: "plan-points",
                    for (index, form) in form_list.iter().enumerate() {
                        div { key: "{index}", class: "plan-point",
                            span { class: "plan-point-index", "{index + 1}" }
                            label { class: "field-label", "Depth ({prefs.depth_unit()})"
                                input {
                                    class: "field",
                                    r#type: "number",
                                    value: "{form.depth}",
                                    oninput: move |evt| forms.write()[index].depth = evt.value().parse().unwrap_or(0.0),
                                }
                            }
                            label { class: "field-label", "Duration (min)"
                                input {
                                    class: "field",
                                    r#type: "number",
                                    value: "{form.minutes}",
                                    oninput: move |evt| forms.write()[index].minutes = evt.value().parse().unwrap_or(0.0),
                                }
                            }
                            label { class: "field-label", "Run time"
                                div { class: "readout", "{format_duration(Duration::new((run_seconds[index] * 60.0).round() as i32))}" }
                            }
                            label { class: "field-label", "O2 %"
                                input {
                                    class: "field",
                                    r#type: "number",
                                    value: "{form.o2}",
                                    oninput: move |evt| forms.write()[index].o2 = evt.value().parse().unwrap_or(21.0),
                                }
                            }
                            label { class: "field-label", "He %"
                                input {
                                    class: "field",
                                    r#type: "number",
                                    value: "{form.he}",
                                    oninput: move |evt| forms.write()[index].he = evt.value().parse().unwrap_or(0.0),
                                }
                            }
                            label { class: "field-label", "Dive mode"
                                select {
                                    class: "field",
                                    value: "{form.mode_index}",
                                    onchange: move |evt| forms.write()[index].mode_index = evt.value().parse().unwrap_or(0),
                                    option { value: "0", "OC" }
                                    option { value: "1", "CCR" }
                                    option { value: "2", "pSCR" }
                                }
                            }
                            if form.mode_index == 1 {
                                label { class: "field-label", "Setpoint"
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
                                label { class: "field-label", "Dump ratio"
                                    input {
                                        class: "field",
                                        r#type: "number",
                                        step: "10",
                                        value: "{form.dump_ratio}",
                                        oninput: move |evt| forms.write()[index].dump_ratio = evt.value().parse().unwrap_or(100.0),
                                    }
                                }
                            }
                            label { class: "field-label", "Used gas"
                                div { class: "readout", "{used_liters[index]:.0} L" }
                            }
                            button {
                                class: "btn point-remove",
                                title: "Remove this waypoint",
                                disabled: form_list.len() <= 1,
                                onclick: move |_| { forms.write().remove(index); },
                                "\u{00d7}"
                            }
                        }
                    }
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
                    "+ Add waypoint"
                }

                div { class: "section-title", "Settings" }
                div { class: "edit-form",
                    label { class: "field-label", "Deco model"
                        select {
                            class: "field",
                            value: "{deco_model_value}",
                            onchange: move |evt| deco_model_index.set(evt.value().parse().unwrap_or(0)),
                            option { value: "0", "B\u{fc}hlmann (GF)" }
                            option { value: "1", "VPM-B" }
                        }
                    }
                    if deco_model_value == 1 {
                        label { class: "field-label", "Conservatism"
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
                        label { class: "field-label", "GF low"
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
                        label { class: "field-label", "GF high"
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
                    label { class: "field-label", "RMV (L/min)"
                        input {
                            class: "field",
                            r#type: "number",
                            value: "{rmv}",
                            oninput: move |evt| rmv.set(evt.value().parse().unwrap_or(20.0)),
                        }
                    }
                }
                if over_mod {
                    p { class: "warn", "Warning: a waypoint exceeds the bottom gas MOD at a 1.4 bar pO2 limit." }
                }

                div { class: "section-title", "Summary" }
                div { class: "facts planner-results",
                    Result { label: gas_label, value: bottom_gas.name() }
                    Result { label: "Ambient", value: format!("{ambient:.2} bar") }
                    Result { label: "MOD (1.4)", value: prefs.depth(Depth::new(mod_mm)) }
                    Result { label: "END", value: prefs.depth(Depth::new(end_mm)) }
                    Result { label: "NDL", value: ndl_text }
                    Result { label: "Runtime", value: format_duration(plan.total_time()) }
                    Result { label: "Bottom time", value: format_duration(plan.bottom_time()) }
                    Result { label: "Deco time", value: format_duration(plan.deco_time()) }
                    if !rebreather {
                        Result { label: "Gas needed", value: format!("{gas_needs:.0} L") }
                        Result { label: "\u{2248} 12 L fills", value: format!("{gas_bar_12l:.0} bar") }
                    }
                    Result { label: "OC bailout", value: format!("{bailout:.0} L") }
                    Result { label: "\u{2248} 12 L bailout", value: format!("{bailout_bar_12l:.0} bar") }
                }

                if has_stops {
                    div { class: "section-title", "Decompression schedule" }
                    table { class: "data-table",
                        thead { tr { th { "Stop" } th { "Time" } } }
                        tbody {
                            for (stop_depth, stop_time) in stops {
                                tr { td { "{stop_depth}" } td { "{stop_time}" } }
                            }
                        }
                    }
                } else {
                    p { class: "muted", "No decompression stops required." }
                }
            }
        }
    }
}

#[component]
fn Result(label: &'static str, value: String) -> Element {
    rsx! {
        div { class: "fact",
            span { class: "fact-label", "{label}" }
            span { class: "fact-value", "{value}" }
        }
    }
}
