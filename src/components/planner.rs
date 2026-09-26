use dioxus::prelude::*;

use benthic_core::gas::{
    ambient_mbar, end_depth_mm, mod_depth_mm, GasMix, DEFAULT_PO2_LIMIT_MBAR, SURFACE_PRESSURE_MBAR,
};
use benthic_core::units::{format_duration, Depth};
use benthic_core::Buhlmann;

use crate::state::AppState;

/// A simple Bühlmann planner: pick a depth, bottom time and gas and see the
/// NDL, the ceiling after the planned bottom time, and the gas limits.
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
    let mut gf = use_signal(|| 1.0f64);

    let target_depth = prefs.depth_from_value((depth)());
    let gas = GasMix::percent((o2)(), (he)());
    let gf_value = (gf)();

    let model = Buhlmann::new(SURFACE_PRESSURE_MBAR / 1000.0, salinity);
    let ndl = model.ndl(target_depth, gas, gf_value);
    let mut after_bottom = Buhlmann::new(SURFACE_PRESSURE_MBAR / 1000.0, salinity);
    after_bottom.add_segment_minutes(target_depth, (bottom)(), gas);
    let ceiling = after_bottom.ceiling_depth(gf_value);

    let mod_mm = mod_depth_mm(gas, DEFAULT_PO2_LIMIT_MBAR, SURFACE_PRESSURE_MBAR, salinity);
    let end_mm = end_depth_mm(gas, target_depth.mm, SURFACE_PRESSURE_MBAR, salinity, false);
    let over_mod = target_depth.mm > mod_mm;
    let ambient = ambient_mbar(target_depth.mm, SURFACE_PRESSURE_MBAR, salinity) / 1000.0;

    let ndl_text = ndl
        .map(|d| {
            if d.seconds >= 24 * 3600 {
                "> 24 h".to_string()
            } else {
                format_duration(d)
            }
        })
        .unwrap_or_else(|| "> 24 h".to_string());
    let ceiling_text = if ceiling.mm > 0 {
        prefs.depth(ceiling)
    } else {
        "none".to_string()
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
                            value: "{depth_value_string(&state, target_depth)}",
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
                    label { class: "field-label", "Gradient factor"
                        input {
                            class: "field",
                            r#type: "number",
                            min: "0.1",
                            max: "1.0",
                            step: "0.05",
                            value: "{gf_value}",
                            oninput: move |evt| gf.set(evt.value().parse::<f64>().unwrap_or(1.0).clamp(0.1, 1.0)),
                        }
                    }
                }
                div { class: "facts planner-results",
                    Result { label: "Gas", value: gas.name() }
                    Result { label: "Ambient", value: format!("{ambient:.2} bar") }
                    Result { label: "MOD (1.4)", value: prefs.depth(Depth::new(mod_mm)) }
                    Result { label: "END", value: prefs.depth(Depth::new(end_mm)) }
                    Result { label: "NDL", value: ndl_text }
                    Result {
                        label: "Ceiling after bottom time",
                        value: ceiling_text,
                    }
                }
                if over_mod {
                    p { class: "warn", "Warning: depth exceeds the gas MOD at a 1.4 bar pO2 limit." }
                }
                div { class: "detail-actions",
                    button {
                        class: "btn primary",
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

fn depth_value_string(state: &AppState, depth: benthic_core::Depth) -> String {
    format!("{:.1}", (state.prefs)().depth_value(depth))
}
