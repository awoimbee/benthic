use dioxus::prelude::*;

use benthic_core::deco::{BreathingMode, PscrParams};
use benthic_core::gas::{
    ambient_mbar, end_depth_mm, mod_depth_mm, GasMix, DEFAULT_PO2_LIMIT_MBAR, SURFACE_PRESSURE_MBAR,
};
use benthic_core::units::{format_duration, Depth, Duration};
use benthic_core::{DecoModel, Dive, DiveComputer, DivePlan};

use crate::actions;
use crate::components::DiveProfile;
use crate::state::AppState;

/// A Bühlmann planner: pick a depth, bottom time, breathing mode and gas and
/// see the NDL, the decompression schedule, the gas limits and the gas needs.
#[component]
pub fn PlannerDialog() -> Element {
    let state = use_context::<AppState>();
    let mut show_planner = state.show_planner;
    let prefs = (state.prefs)();
    let salinity = (state.prefs)().default_salinity.value();

    let mut depth = use_signal(|| 30.0f64);
    let mut bottom = use_signal(|| 20.0f64);
    let mut o2 = use_signal(|| 21.0f64);
    let mut he = use_signal(|| 0.0f64);
    let mut mode_index = use_signal(|| 0usize);
    let mut setpoint = use_signal(|| 1.3f64);
    let mut dump_ratio = use_signal(|| 100.0f64);
    let mut gf_low = use_signal(|| 0.30f64);
    let mut gf_high = use_signal(|| 0.70f64);
    let mut deco_model_index = use_signal(|| 0usize);
    let mut vpmb_conservatism = use_signal(|| 3u8);
    let mut rmv = use_signal(|| 20.0f64);

    let target_depth = prefs.depth_from_value((depth)());
    let bottom_time = Duration::from_minutes((bottom)().round().max(0.0) as i32);
    let diluent = GasMix::percent((o2)(), (he)());
    let mode_value = (mode_index)();
    let setpoint_value = (setpoint)();
    let dump_ratio_value = (dump_ratio)().max(1.0);
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

    let mode = match mode_value {
        1 => BreathingMode::ClosedCircuit {
            diluent,
            setpoint_bar: setpoint_value,
        },
        2 => BreathingMode::PassiveSemiClosed {
            diluent,
            params: PscrParams {
                dump_ratio: dump_ratio_value,
                ..Default::default()
            },
        },
        _ => BreathingMode::OpenCircuit(diluent),
    };
    let rebreather = mode.is_rebreather();

    let plan = DivePlan::compute(
        target_depth,
        bottom_time,
        mode,
        deco_model,
        SURFACE_PRESSURE_MBAR / 1000.0,
        salinity,
    );
    let ndl = plan.ndl(SURFACE_PRESSURE_MBAR / 1000.0, salinity, gf_high_value);

    let mod_mm = mod_depth_mm(
        diluent,
        DEFAULT_PO2_LIMIT_MBAR,
        SURFACE_PRESSURE_MBAR,
        salinity,
    );
    let gas_needs = plan.gas_needs_liters((rmv)(), SURFACE_PRESSURE_MBAR / 1000.0, salinity);
    let gas_bar_12l = gas_needs / 12.0;
    let bailout = plan.bailout_liters((rmv)(), SURFACE_PRESSURE_MBAR / 1000.0, salinity);
    let bailout_bar_12l = bailout / 12.0;
    let end_mm = end_depth_mm(
        diluent,
        target_depth.mm,
        SURFACE_PRESSURE_MBAR,
        salinity,
        false,
    );
    let over_mod = target_depth.mm > mod_mm;
    let ambient = ambient_mbar(target_depth.mm, SURFACE_PRESSURE_MBAR, salinity) / 1000.0;
    let depth_string = format!("{:.1}", prefs.depth_value(target_depth));
    let gas_label: &'static str = if rebreather { "Diluent" } else { "Gas" };

    let ndl_text = ndl
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

    // The plan's own profile, drawn with the same interactive chart as a real
    // dive: descent, bottom time, the ascent and each deco stop plateau.
    let graph_dive = Dive {
        max_depth: Some(target_depth),
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
        "{} for {} \u{b7} {} \u{b7} {deco_label}",
        prefs.depth(target_depth),
        format_duration(bottom_time),
        diluent.name(),
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

                div { class: "section-title", "Settings" }
                div { class: "edit-form",
                    label { class: "field-label", "Depth ({prefs.depth_unit()})"
                        input {
                            class: "field",
                            r#type: "number",
                            value: "{depth_string}",
                            oninput: move |evt| depth.set(evt.value().parse().unwrap_or(0.0)),
                        }
                    }
                    label { class: "field-label", "Bottom time (min)"
                        input {
                            class: "field",
                            r#type: "number",
                            value: "{bottom}",
                            oninput: move |evt| bottom.set(evt.value().parse().unwrap_or(0.0)),
                        }
                    }
                    label { class: "field-label", "O2 %"
                        input {
                            class: "field",
                            r#type: "number",
                            value: "{o2}",
                            oninput: move |evt| o2.set(evt.value().parse().unwrap_or(21.0)),
                        }
                    }
                    label { class: "field-label", "He %"
                        input {
                            class: "field",
                            r#type: "number",
                            value: "{he}",
                            oninput: move |evt| he.set(evt.value().parse().unwrap_or(0.0)),
                        }
                    }
                    label { class: "field-label", "Mode"
                        select {
                            class: "field",
                            value: "{mode_value}",
                            onchange: move |evt| mode_index.set(evt.value().parse().unwrap_or(0)),
                            option { value: "0", "Open circuit" }
                            option { value: "1", "CCR" }
                            option { value: "2", "pSCR" }
                        }
                    }
                    if mode_value == 1 {
                        label { class: "field-label", "Setpoint (bar)"
                            input {
                                class: "field",
                                r#type: "number",
                                step: "0.1",
                                value: "{setpoint_value}",
                                oninput: move |evt| setpoint.set(evt.value().parse().unwrap_or(1.3)),
                            }
                        }
                    }
                    if mode_value == 2 {
                        label { class: "field-label", "Dump ratio"
                            input {
                                class: "field",
                                r#type: "number",
                                step: "10",
                                value: "{dump_ratio_value}",
                                oninput: move |evt| dump_ratio.set(evt.value().parse::<f64>().unwrap_or(100.0).max(1.0)),
                            }
                        }
                    }
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
                    p { class: "warn", "Warning: depth exceeds the diluent MOD at a 1.4 bar pO2 limit." }
                }

                div { class: "section-title", "Summary" }
                div { class: "facts planner-results",
                    Result { label: gas_label, value: diluent.name() }
                    Result { label: "Ambient", value: format!("{ambient:.2} bar") }
                    Result { label: "MOD (1.4)", value: prefs.depth(Depth::new(mod_mm)) }
                    Result { label: "END", value: prefs.depth(Depth::new(end_mm)) }
                    Result { label: "NDL", value: ndl_text }
                    Result { label: "Runtime", value: format_duration(plan.total_time()) }
                    Result { label: "Deco time", value: format_duration(plan.deco_time()) }
                    if mode_value == 1 {
                        Result { label: "Setpoint", value: format!("{setpoint_value:.1} bar") }
                    }
                    if mode_value == 2 {
                        Result { label: "Fresh gas", value: format!("1:{:.0}", 1000.0 / dump_ratio_value) }
                    }
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
