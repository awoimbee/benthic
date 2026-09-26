use dioxus::prelude::*;

use benthic_core::deco::BreathingMode;
use benthic_core::gas::{
    ambient_mbar, end_depth_mm, mod_depth_mm, GasMix, DEFAULT_PO2_LIMIT_MBAR, SURFACE_PRESSURE_MBAR,
};
use benthic_core::units::{format_duration, Depth, Duration};
use benthic_core::{Buhlmann, DivePlan};

use crate::actions;
use crate::state::AppState;

/// A Bühlmann planner: pick a depth, bottom time and gas and see the NDL, the
/// decompression schedule and the gas limits.
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
    let mut gf_low = use_signal(|| 0.30f64);
    let mut gf_high = use_signal(|| 0.70f64);
    let mut rmv = use_signal(|| 20.0f64);
    let mut ccr = use_signal(|| false);
    let mut setpoint = use_signal(|| 1.3f64);

    let target_depth = prefs.depth_from_value((depth)());
    let bottom_time = Duration::from_minutes((bottom)().round().max(0.0) as i32);
    let diluent = GasMix::percent((o2)(), (he)());
    let gf_low_value = (gf_low)();
    let gf_high_value = (gf_high)();
    let ccr_on = (ccr)();
    let setpoint_value = (setpoint)();
    let mode = if ccr_on {
        BreathingMode::ClosedCircuit {
            diluent,
            setpoint_bar: setpoint_value,
        }
    } else {
        BreathingMode::OpenCircuit(diluent)
    };

    let model = Buhlmann::new(SURFACE_PRESSURE_MBAR / 1000.0, salinity);
    let ndl = if ccr_on {
        model.ndl_ccr(target_depth, diluent, setpoint_value, gf_high_value)
    } else {
        model.ndl(target_depth, diluent, gf_high_value)
    };
    let plan = DivePlan::compute(
        target_depth,
        bottom_time,
        mode,
        gf_low_value,
        gf_high_value,
        SURFACE_PRESSURE_MBAR / 1000.0,
        salinity,
    );

    let mod_mm = mod_depth_mm(
        diluent,
        DEFAULT_PO2_LIMIT_MBAR,
        SURFACE_PRESSURE_MBAR,
        salinity,
    );
    let gas_needs = plan.gas_needs_liters((rmv)(), SURFACE_PRESSURE_MBAR / 1000.0, salinity);
    let gas_bar_12l = gas_needs / 12.0;
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
    let gas_label: &'static str = if ccr_on { "Diluent" } else { "Gas" };

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

    let plan_for_save = plan.clone();
    let on_save = move |_| {
        actions::save_plan(state, plan_for_save.clone());
        show_planner.set(false);
    };

    rsx! {
        div {
            class: "modal-backdrop",
            onclick: move |_| show_planner.set(false),
            div {
                class: "modal wide",
                onclick: move |evt| evt.stop_propagation(),
                h2 { "Dive planner" }
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
                    label { class: "field-label", "Closed circuit"
                        div { class: "check",
                            input {
                                r#type: "checkbox",
                                checked: ccr_on,
                                onchange: move |_| ccr.set(!ccr_on),
                            }
                            span { "CCR (diluent + setpoint)" }
                        }
                    }
                    if ccr_on {
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
                    label { class: "field-label", "RMV (L/min)"
                        input {
                            class: "field",
                            r#type: "number",
                            value: "{rmv}",
                            oninput: move |evt| rmv.set(evt.value().parse().unwrap_or(20.0)),
                        }
                    }
                }
                div { class: "facts planner-results",
                    Result { label: gas_label, value: diluent.name() }
                    Result { label: "Ambient", value: format!("{ambient:.2} bar") }
                    Result { label: "MOD (1.4)", value: prefs.depth(Depth::new(mod_mm)) }
                    Result { label: "END", value: prefs.depth(Depth::new(end_mm)) }
                    Result { label: "NDL", value: ndl_text }
                    Result { label: "Runtime", value: format_duration(plan.total_time()) }
                    Result { label: "Deco time", value: format_duration(plan.deco_time()) }
                    if ccr_on {
                        Result { label: "Setpoint", value: format!("{setpoint_value:.1} bar") }
                    } else {
                        Result { label: "Gas needed", value: format!("{gas_needs:.0} L") }
                        Result { label: "≈ 12 L fills", value: format!("{gas_bar_12l:.0} bar") }
                    }
                }
                if over_mod {
                    p { class: "warn", "Warning: depth exceeds the gas MOD at a 1.4 bar pO2 limit." }
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
                div { class: "detail-actions",
                    button { class: "btn primary", onclick: on_save, "Save as dive" }
                    button {
                        class: "btn",
                        onclick: move |_| show_planner.set(false),
                        "Close"
                    }
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
