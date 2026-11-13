use dioxus::prelude::*;

use benthic_core::deco::{BreathingMode, PlanPoint, PscrParams};
use benthic_core::gas::{ambient_mbar, end_depth_mm, mod_depth_mm, GasMix, SURFACE_PRESSURE_MBAR};
use benthic_core::planner::{ndl as plan_ndl, GasSupply};
use benthic_core::units::{format_duration, Depth, Duration, Pressure, Volume};
use benthic_core::{DecoModel, Dive, DiveComputer, DivePlan, Preferences};

use crate::actions;
use crate::components::{DiveProfile, InfoTip};
use crate::i18n;
use crate::routing::{self, Route};
use crate::state::{AppState, Dialog};

/// Cylinder colours, distinct from the depth and ceiling traces.
const GAS_COLORS: [&str; 8] = [
    "#7ee787", "#f5a623", "#f28fad", "#ffd166", "#ff7b72", "#79c0ff", "#d0ffb7", "#e3b341",
];

fn gas_color(gases: &[GasForm], gas: GasMix) -> &'static str {
    let index = gases
        .iter()
        .position(|candidate| candidate.gas() == gas)
        .unwrap_or(0);
    GAS_COLORS[index % GAS_COLORS.len()]
}

/// One editable cylinder.
#[derive(Clone, PartialEq)]
struct GasForm {
    size: f64,
    pressure: f64,
    o2: f64,
    he: f64,
    enabled: bool,
}

impl Default for GasForm {
    fn default() -> Self {
        Self {
            size: 15.0,
            pressure: 200.0,
            o2: 21.0,
            he: 0.0,
            enabled: true,
        }
    }
}

impl GasForm {
    fn gas(&self) -> GasMix {
        GasMix::percent(self.o2, self.he)
    }

    fn supply(&self) -> GasSupply {
        GasSupply {
            gas: self.gas(),
            size: Volume::from_liters(self.size.max(0.0)),
            start_pressure: Pressure::from_bar(self.pressure.max(0.0)),
        }
    }

    fn switch_depth(&self, max_ppo2: f64, safety_m: f64, salinity: i32) -> Depth {
        let mod_mm = mod_depth_mm(
            self.gas(),
            (max_ppo2.max(0.1) * 1000.0).round(),
            SURFACE_PRESSURE_MBAR,
            salinity,
        );
        let usable_mm = (mod_mm - (safety_m * 1000.0).round() as i32).max(0);
        Depth::new((usable_mm / 1000) * 1000)
    }
}

/// One editable waypoint of the optional custom profile.
#[derive(Clone, PartialEq)]
struct PointForm {
    depth: f64,
    minutes: f64,
}

impl Default for PointForm {
    fn default() -> Self {
        Self {
            depth: 30.0,
            minutes: 20.0,
        }
    }
}

impl PointForm {
    fn to_plan_point(&self, prefs: &Preferences, mode: BreathingMode) -> PlanPoint {
        PlanPoint {
            depth: prefs.depth_from_value(self.depth),
            duration: Duration::from_minutes(self.minutes.round().max(0.0) as i32),
            mode,
        }
    }
}

/// Everything the planner can restore from a shareable URL.
#[derive(Clone)]
struct PlannerParams {
    gases: Vec<GasForm>,
    depth: f64,
    minutes: f64,
    custom: bool,
    points: Vec<PointForm>,
    mode_index: usize,
    setpoint: f64,
    dump_ratio: f64,
    max_ppo2: f64,
    switch_safety: f64,
    reserve: f64,
    gf_low: f64,
    gf_high: f64,
    model_index: usize,
    conservatism: u8,
    rmv: f64,
}

impl PlannerParams {
    fn from_query(prefs: &Preferences, query: Option<&str>) -> Self {
        let defaults = Self::defaults(prefs);
        let Some(query) = query else {
            return defaults;
        };
        let values = parse_query(query);
        let value = |key: &str| values.get(key).map(String::as_str);
        let number = |key: &str| value(key).and_then(|v| v.parse::<f64>().ok());

        let mut params = defaults.clone();
        params.mode_index = match value("mode") {
            Some("ccr") => 1,
            Some("pscr") => 2,
            _ => 0,
        };
        params.setpoint = number("setpoint").unwrap_or(params.setpoint);
        params.dump_ratio = number("dump").unwrap_or(params.dump_ratio);
        params.max_ppo2 = number("ppo2").unwrap_or(params.max_ppo2).clamp(1.4, 2.0);
        params.depth = number("depth")
            .map(|m| prefs.depth_value(Depth::from_meters(m)))
            .unwrap_or(params.depth);
        params.minutes = number("time").unwrap_or(params.minutes);
        params.custom = value("custom").is_some_and(|v| v == "1" || v == "true");
        params.reserve = number("reserve").unwrap_or(params.reserve).max(0.0);
        params.rmv = number("rmv").unwrap_or(params.rmv).max(1.0);
        params.model_index = if value("model") == Some("vpmb") { 1 } else { 0 };
        params.conservatism = number("conservatism")
            .unwrap_or(params.conservatism as f64)
            .round()
            .clamp(0.0, 4.0) as u8;
        params.gf_low = parse_gf(number("gflow"), params.gf_low);
        params.gf_high = parse_gf(number("gfhigh"), params.gf_high);
        params.switch_safety = number("safety")
            .map(|m| prefs.depth_value(Depth::from_meters(m)))
            .unwrap_or(params.switch_safety);

        let gas_count = value("gases")
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or_else(|| {
                let mut count = 0;
                while value(&format!("gas{count}vol")).is_some()
                    || value(&format!("gas{count}bar")).is_some()
                    || value(&format!("gas{count}o2")).is_some()
                    || value(&format!("gas{count}he")).is_some()
                {
                    count += 1;
                }
                count
            })
            .clamp(1, 10);
        params.gases = (0..gas_count)
            .map(|index| {
                let default = GasForm::default();
                let o2 = number(&format!("gas{index}o2"))
                    .unwrap_or(default.o2)
                    .clamp(21.0, 100.0);
                GasForm {
                    size: number(&format!("gas{index}vol"))
                        .unwrap_or(default.size)
                        .clamp(1.0, 38.0),
                    pressure: number(&format!("gas{index}bar"))
                        .unwrap_or(default.pressure)
                        .clamp(0.0, 300.0),
                    o2,
                    he: number(&format!("gas{index}he"))
                        .unwrap_or(default.he)
                        .clamp(0.0, 100.0 - o2),
                    enabled: value(&format!("gas{index}on"))
                        .map(|v| v != "0" && v != "false")
                        .unwrap_or(true),
                }
            })
            .collect();
        if params.gases.iter().all(|gas| !gas.enabled) {
            if let Some(gas) = params.gases.first_mut() {
                gas.enabled = true;
            }
        }

        if params.custom {
            let waypoint_count = value("waypoints")
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or_else(|| {
                    let mut count = 0;
                    while value(&format!("wp{count}depth")).is_some()
                        || value(&format!("wp{count}time")).is_some()
                    {
                        count += 1;
                    }
                    count
                })
                .clamp(1, 50);
            let mut points = Vec::new();
            for index in 0..waypoint_count {
                let fallback = params.points.get(index).cloned().unwrap_or_default();
                points.push(PointForm {
                    depth: number(&format!("wp{index}depth"))
                        .map(|m| prefs.depth_value(Depth::from_meters(m)))
                        .unwrap_or(fallback.depth)
                        .max(0.0),
                    minutes: number(&format!("wp{index}time"))
                        .unwrap_or(fallback.minutes)
                        .max(0.0),
                });
            }
            params.points = points;
        }
        params
    }

    fn defaults(prefs: &Preferences) -> Self {
        let depth = prefs.depth_value(Depth::from_meters(30.0));
        let minutes = 25.0;
        Self {
            gases: vec![GasForm::default()],
            depth,
            minutes,
            custom: false,
            points: square_points(depth, minutes),
            mode_index: 0,
            setpoint: 1.3,
            dump_ratio: 100.0,
            max_ppo2: 1.6,
            switch_safety: prefs.depth_value(Depth::from_meters(1.0)),
            reserve: 50.0,
            gf_low: 0.85,
            gf_high: 0.85,
            model_index: 0,
            conservatism: 3,
            rmv: 20.0,
        }
    }
}

fn parse_gf(value: Option<f64>, fallback: f64) -> f64 {
    match value {
        Some(value) if value > 1.0 => (value / 100.0).clamp(0.1, 1.0),
        Some(value) => value.clamp(0.1, 1.0),
        None => fallback,
    }
}

fn parse_query(query: &str) -> std::collections::HashMap<String, String> {
    query
        .trim_start_matches('?')
        .split('&')
        .filter_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            Some((key.to_string(), value.to_string()))
        })
        .collect()
}

fn square_points(depth: f64, minutes: f64) -> Vec<PointForm> {
    let descent = if depth > 0.0 {
        (depth / 20.0 * 60.0).round()
    } else {
        0.0
    };
    vec![
        PointForm {
            depth,
            minutes: descent,
        },
        PointForm { depth, minutes },
    ]
}

#[allow(clippy::too_many_arguments)]
fn share_query(
    prefs: &Preferences,
    mode_index: usize,
    gases: &[GasForm],
    depth: f64,
    minutes: f64,
    custom: bool,
    points: &[PointForm],
    setpoint: f64,
    dump_ratio: f64,
    max_ppo2: f64,
    switch_safety: f64,
    reserve: f64,
    gf_low: f64,
    gf_high: f64,
    model_index: usize,
    conservatism: u8,
    rmv: f64,
) -> String {
    let mut parts = vec![
        format!(
            "mode={}",
            match mode_index {
                1 => "ccr",
                2 => "pscr",
                _ => "oc",
            }
        ),
        format!("gases={}", gases.len()),
    ];
    for (index, gas) in gases.iter().enumerate() {
        parts.push(format!("gas{index}vol={}", number(gas.size)));
        parts.push(format!("gas{index}bar={}", number(gas.pressure)));
        parts.push(format!("gas{index}o2={}", number(gas.o2)));
        parts.push(format!("gas{index}he={}", number(gas.he)));
        parts.push(format!("gas{index}on={}", if gas.enabled { 1 } else { 0 }));
    }
    parts.push(format!(
        "depth={}",
        number(prefs.depth_from_value(depth).meters())
    ));
    parts.push(format!("time={}", number(minutes)));
    parts.push(format!("custom={}", if custom { 1 } else { 0 }));
    if custom {
        parts.push(format!("waypoints={}", points.len()));
        for (index, point) in points.iter().enumerate() {
            parts.push(format!(
                "wp{index}depth={}",
                number(prefs.depth_from_value(point.depth).meters())
            ));
            parts.push(format!("wp{index}time={}", number(point.minutes)));
        }
    }
    parts.push(format!("setpoint={}", number(setpoint)));
    parts.push(format!("dump={}", number(dump_ratio)));
    parts.push(format!("ppo2={}", number(max_ppo2)));
    parts.push(format!(
        "safety={}",
        number(prefs.depth_from_value(switch_safety).meters())
    ));
    parts.push(format!("reserve={}", number(reserve)));
    parts.push(format!("rmv={}", number(rmv)));
    parts.push(format!(
        "model={}",
        if model_index == 1 { "vpmb" } else { "gf" }
    ));
    parts.push(format!("gflow={}", number((gf_low * 100.0).round())));
    parts.push(format!("gfhigh={}", number((gf_high * 100.0).round())));
    parts.push(format!("conservatism={conservatism}"));
    parts.join("&")
}

fn number(value: f64) -> String {
    if !value.is_finite() {
        return "0".to_string();
    }
    if (value.fract()).abs() < 1e-9 {
        format!("{value:.0}")
    } else {
        let mut out = format!("{value:.2}");
        while out.ends_with('0') {
            out.pop();
        }
        if out.ends_with('.') {
            out.pop();
        }
        out
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

/// A circular value control. Drag vertically to adjust; tap/click or press
/// Enter to type an exact value. Designed for coarse pointers first.
#[component]
fn Gauge(
    label: &'static str,
    unit: &'static str,
    hint: &'static str,
    value: f64,
    min: f64,
    max: f64,
    step: f64,
    onchange: EventHandler<f64>,
    #[props(default)] decimals: usize,
    #[props(default)] big: bool,
    #[props(default)] disabled: bool,
) -> Element {
    let mut editing = use_signal(|| false);
    let mut drag = use_signal(|| None::<(f64, f64, bool)>);
    let mut anchor = use_signal(|| (0.0f64, 0.0f64));
    let mut gauge_rect = use_signal(|| None::<(f64, f64, f64, f64)>);
    let fraction = if max > min {
        ((value - min) / (max - min)).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let shown = format!("{:.*}", decimals, value);
    let progress = fraction * 100.0;
    let quantize = move |next: f64| {
        let step = step.max(0.001);
        let snapped = min + ((next - min) / step).round() * step;
        snapped.clamp(min, max)
    };
    let mut class = String::from("planner-gauge");
    if big {
        class.push_str(" big");
    }
    if disabled {
        class.push_str(" disabled");
    }
    let step_size = step.max(0.001);
    let option_count = (((max - min) / step_size).round() as usize).min(5000);
    let options: Vec<(f64, String)> = (0..=option_count)
        .map(|index| {
            let option = min + index as f64 * step_size;
            (option, format!("{:.*}", decimals, option))
        })
        .collect();
    let selected_index = options
        .iter()
        .position(|(option, _)| (option - value).abs() <= step_size / 2.0)
        .unwrap_or(0);
    let (anchor_x, anchor_y) = (anchor)();
    let picker_style = format!(
        "left: clamp(0.5rem, calc({anchor_x}px - 9.5rem), calc(100vw - 19.5rem)); top: clamp(0.5rem, calc({anchor_y}px + 0.5rem), calc(100dvh - 29rem));"
    );

    rsx! {
        div {
            class: "{class}",
            role: "slider",
            tabindex: "0",
            title: "{hint}",
            aria_label: "{label}",
            aria_valuemin: "{min}",
            aria_valuemax: "{max}",
            aria_valuenow: "{value}",
            aria_disabled: if disabled { "true" } else { "false" },
            aria_haspopup: "dialog",
            onmounted: move |evt: MountedEvent| async move {
                if let Ok(rect) = evt.get_client_rect().await {
                    gauge_rect.set(Some((
                        rect.origin.x,
                        rect.origin.y,
                        rect.size.width,
                        rect.size.height,
                    )));
                }
            },
            onkeydown: move |evt: KeyboardEvent| {
                if disabled {
                    return;
                }
                match evt.key() {
                Key::ArrowUp | Key::ArrowRight => {
                    onchange.call(quantize(value + step));
                    evt.prevent_default();
                }
                Key::ArrowDown | Key::ArrowLeft => {
                    onchange.call(quantize(value - step));
                    evt.prevent_default();
                }
                Key::Enter => {
                    if let Some((x, y, width, height)) = (gauge_rect)() {
                        anchor.set((x + width / 2.0, y + height));
                    }
                    editing.set(true);
                    evt.prevent_default();
                }
                Key::Escape => editing.set(false),
                _ => {}
                }
            },
            onpointerdown: move |evt: PointerEvent| {
                if disabled || editing() {
                    return;
                }
                drag.set(Some((evt.client_coordinates().y, value, false)));
            },
            onpointermove: move |evt: PointerEvent| {
                if disabled {
                    return;
                }
                let Some((start_y, start_value, moved)) = (drag)() else {
                    return;
                };
                // Client coordinates stay stable when the pointer moves over
                // the child text/labels, unlike element-relative coordinates.
                let delta = start_y - evt.client_coordinates().y;
                if !moved && delta.abs() < 6.0 {
                    return;
                }
                onchange.call(quantize(start_value + delta / 140.0 * (max - min)));
                drag.set(Some((start_y, start_value, true)));
            },
            onpointerup: move |evt: PointerEvent| {
                if disabled {
                    return;
                }
                let Some((_, _, moved)) = (drag)() else {
                    return;
                };
                drag.set(None);
                if !moved {
                    let coords = evt.client_coordinates();
                    anchor.set((coords.x, coords.y));
                    editing.set(true);
                }
            },
            onpointercancel: move |_| drag.set(None),
            svg {
                class: "planner-gauge-ring",
                view_box: "0 0 120 120",
                path {
                    class: "planner-gauge-track",
                    d: "M 30 100 A 50 50 0 1 1 90 100",
                    path_length: "100",
                }
                path {
                    class: "planner-gauge-progress",
                    d: "M 30 100 A 50 50 0 1 1 90 100",
                    path_length: "100",
                    stroke_dasharray: "{progress:.2} 100",
                }
            }
            div { class: "planner-gauge-content",
                span { class: "planner-gauge-value", "{shown}" }
                span { class: "planner-gauge-unit", "{unit}" }
                span { class: "planner-gauge-label", "{label}" }
            }
            // While dragging, this full-viewport layer keeps receiving moves
            // even when the pointer leaves the gauge. Touch has implicit
            // pointer capture, so its events continue to arrive at the gauge
            // itself; mouse uses this layer.
            if (drag)().is_some() {
                div {
                    class: "planner-gauge-drag-layer",
                    onpointermove: move |evt: PointerEvent| {
                        let Some((start_y, start_value, moved)) = (drag)() else {
                            return;
                        };
                        let delta = start_y - evt.client_coordinates().y;
                        if !moved && delta.abs() < 6.0 {
                            return;
                        }
                        onchange.call(quantize(start_value + delta / 140.0 * (max - min)));
                        drag.set(Some((start_y, start_value, true)));
                    },
                    onpointerup: move |evt: PointerEvent| {
                        let Some((_, _, moved)) = (drag)() else {
                            return;
                        };
                        drag.set(None);
                        if !moved {
                            let coords = evt.client_coordinates();
                            anchor.set((coords.x, coords.y));
                            editing.set(true);
                        }
                    },
                    onpointercancel: move |_| drag.set(None),
                }
            }
        }
        if (editing)() {
            div {
                class: "planner-gauge-picker-backdrop",
                onclick: move |_| editing.set(false),
                div {
                    class: "planner-gauge-picker",
                    style: "{picker_style}",
                    role: "dialog",
                    aria_label: "{label}",
                    onclick: move |evt| evt.stop_propagation(),
                    div { class: "planner-gauge-picker-head",
                        div {
                            div { class: "planner-gauge-picker-title", "{label}" }
                            div { class: "planner-gauge-picker-unit", "{unit}" }
                        }
                        button {
                            class: "icon-btn",
                            r#type: "button",
                            aria_label: "Close",
                            onclick: move |_| editing.set(false),
                            "\u{00d7}"
                        }
                    }
                    div { class: "planner-gauge-picker-list",
                        for (index, (option, option_label)) in options.into_iter().enumerate() {
                            button {
                                key: "{option}",
                                r#type: "button",
                                class: if index == selected_index { "planner-gauge-picker-item selected" } else { "planner-gauge-picker-item" },
                                onclick: move |_| {
                                    onchange.call(option);
                                    editing.set(false);
                                },
                                onmounted: move |evt: MountedEvent| async move {
                                    if index == selected_index {
                                        let _ = evt
                                            .scroll_to_with_options(dioxus::events::ScrollToOptions {
                                                behavior: dioxus::events::ScrollBehavior::Instant,
                                                vertical: dioxus::events::ScrollLogicalPosition::Center,
                                                horizontal: dioxus::events::ScrollLogicalPosition::Center,
                                            })
                                            .await;
                                    }
                                },
                                "{option_label}"
                            }
                        }
                    }
                }
            }
        }
    }
}

/// A dive planner: square profile first, with multiple open-circuit gases and
/// automatic switches, and optional waypoint customisation for advanced plans.
#[component]
pub fn PlannerDialog() -> Element {
    let state = use_context::<AppState>();
    let prefs = (state.prefs)();
    let tr = i18n::strings(prefs.language);
    let salinity = prefs.default_salinity.value();

    let initial = PlannerParams::from_query(&prefs, routing::current_query().as_deref());
    let PlannerParams {
        gases: initial_gases,
        depth: initial_depth,
        minutes: initial_minutes,
        custom: initial_custom,
        points: initial_points,
        mode_index: initial_mode,
        setpoint: initial_setpoint,
        dump_ratio: initial_dump,
        max_ppo2: initial_ppo2,
        switch_safety: initial_safety,
        reserve: initial_reserve,
        gf_low: initial_gf_low,
        gf_high: initial_gf_high,
        model_index: initial_model,
        conservatism: initial_conservatism,
        rmv: initial_rmv,
    } = initial;

    let mut gases = use_signal(move || initial_gases);
    let mut bottom_depth = use_signal(move || initial_depth);
    let mut bottom_minutes = use_signal(move || initial_minutes);
    let mut custom = use_signal(move || initial_custom);
    let mut points = use_signal(move || initial_points);
    let mut mode_index = use_signal(move || initial_mode);
    let mut setpoint = use_signal(move || initial_setpoint);
    let mut dump_ratio = use_signal(move || initial_dump);
    let mut max_ppo2 = use_signal(move || initial_ppo2);
    let mut switch_safety = use_signal(move || initial_safety);
    let mut reserve = use_signal(move || initial_reserve);
    let mut gf_low = use_signal(move || initial_gf_low);
    let mut gf_high = use_signal(move || initial_gf_high);
    let mut deco_model_index = use_signal(move || initial_model);
    let mut vpmb_conservatism = use_signal(move || initial_conservatism);
    let mut rmv = use_signal(move || initial_rmv);
    let mut ndl = use_signal(|| None::<Duration>);
    let mut ndl_key = use_signal(|| None::<NdlKey>);
    let mut expanded = use_signal(|| {
        let mut set = std::collections::HashSet::new();
        set.insert(0usize);
        set
    });

    // Keep the address bar shareable. Parameter edits update the current
    // history entry instead of filling the back stack with every drag tick.
    use_effect(move || {
        let query = share_query(
            &prefs,
            (mode_index)(),
            &(gases)(),
            (bottom_depth)(),
            (bottom_minutes)(),
            (custom)(),
            &(points)(),
            (setpoint)(),
            (dump_ratio)(),
            (max_ppo2)(),
            (switch_safety)(),
            (reserve)(),
            (gf_low)(),
            (gf_high)(),
            (deco_model_index)(),
            (vpmb_conservatism)(),
            (rmv)(),
        );
        routing::replace_query(Route::Dialog(Dialog::Planner), &query);
    });

    let gas_forms = (gases)();
    let active_gases: Vec<GasForm> = gas_forms
        .iter()
        .filter(|gas| gas.enabled)
        .cloned()
        .collect();
    let switch_locked = active_gases
        .iter()
        .filter(|gas| gas.size > 0.0 && gas.pressure > 0.0)
        .count()
        <= 1;
    let gas_supplies: Vec<GasSupply> = active_gases.iter().map(GasForm::supply).collect();
    let max_ppo2_value = (max_ppo2)();
    let safety_m = prefs.depth_from_value((switch_safety)()).meters().max(0.0);
    let reserve_value = Pressure::from_bar((reserve)());
    let mode_value = (mode_index)();
    let setpoint_value = (setpoint)();
    let dump_ratio_value = (dump_ratio)();
    let rmv_value = (rmv)();
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

    // For open circuit, the deepest-capable gas is the bottom gas and the
    // richer gases become automatic ascent switches. For a rebreather, the
    // first gas is the diluent and the mode applies to the whole dive.
    let bottom_gas = active_gases
        .iter()
        .filter(|gas| gas.size > 0.0 && gas.pressure > 0.0)
        .max_by_key(|gas| gas.switch_depth(max_ppo2_value, safety_m, salinity).mm)
        .map(GasForm::gas)
        .unwrap_or_default();
    let global_mode = match mode_value {
        1 => BreathingMode::ClosedCircuit {
            diluent: bottom_gas,
            setpoint_bar: setpoint_value,
        },
        2 => BreathingMode::PassiveSemiClosed {
            diluent: bottom_gas,
            params: PscrParams {
                dump_ratio: dump_ratio_value.max(1.0),
                ..Default::default()
            },
        },
        _ => BreathingMode::OpenCircuit(bottom_gas),
    };

    let custom_value = (custom)();
    let point_forms = (points)();
    let depth_value = (bottom_depth)();
    let minutes_value = (bottom_minutes)();
    let max_depth_gauge = prefs.depth_value(Depth::from_meters(120.0)).max(50.0);
    let max_safety_gauge = prefs.depth_value(Depth::from_meters(5.0)).max(1.0);

    let plan = if custom_value {
        let points: Vec<PlanPoint> = point_forms
            .iter()
            .map(|form| form.to_plan_point(&prefs, global_mode))
            .collect();
        if mode_value == 0 {
            DivePlan::compute_with_gases(
                points,
                &gas_supplies,
                max_ppo2_value,
                Depth::from_meters(safety_m),
                deco_model,
                SURFACE_PRESSURE_MBAR / 1000.0,
                salinity,
                rmv_value,
            )
        } else {
            DivePlan::compute(points, deco_model, SURFACE_PRESSURE_MBAR / 1000.0, salinity)
        }
    } else if mode_value == 0 {
        DivePlan::square_with_gases(
            prefs.depth_from_value(depth_value),
            Duration::from_minutes(minutes_value.round().max(0.0) as i32),
            &gas_supplies,
            max_ppo2_value,
            Depth::from_meters(safety_m),
            deco_model,
            SURFACE_PRESSURE_MBAR / 1000.0,
            salinity,
            rmv_value,
        )
    } else {
        DivePlan::square(
            prefs.depth_from_value(depth_value),
            Duration::from_minutes(minutes_value.round().max(0.0) as i32),
            global_mode,
            deco_model,
            SURFACE_PRESSURE_MBAR / 1000.0,
            salinity,
        )
    };

    // Recompute the NDL only when its inputs change, not on every edit.
    {
        let ndl_base_depth = if custom_value {
            plan.max_depth()
        } else {
            prefs.depth_from_value(depth_value)
        };
        let ndl_gas = if mode_value == 0 {
            BreathingMode::OpenCircuit(bottom_gas)
        } else {
            global_mode
        };
        use_effect(move || {
            let prefs = (state.prefs)();
            let salinity = prefs.default_salinity.value();
            let model_index = (deco_model_index)();
            let conservatism = (vpmb_conservatism)();
            let gf_l = (gf_low)();
            let gf_h = (gf_high)();
            let key = NdlKey {
                depth_mm: ndl_base_depth.mm,
                o2: ndl_gas.cylinder_gas().o2_permille,
                he: ndl_gas.cylinder_gas().he_permille,
                mode_index: (mode_index)(),
                setpoint_100: (setpoint() * 100.0).round() as i32,
                dump_ratio: (dump_ratio)().round() as i32,
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
                ndl_base_depth,
                ndl_gas,
                deco_model,
                SURFACE_PRESSURE_MBAR / 1000.0,
                salinity,
            ));
        });
    }

    let max_depth = plan.max_depth();
    let mod_mm = mod_depth_mm(
        bottom_gas,
        (max_ppo2_value.max(0.1) * 1000.0).round(),
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
    let over_mod = mode_value == 0
        && active_gases.iter().any(|gas| {
            // A gas is used on the bottom only if it is the deepest-capable
            // one; otherwise exceeding its switch depth is fine until the
            // ascent reaches that switch.
            gas.gas() == bottom_gas
                && max_depth.mm > gas.switch_depth(max_ppo2_value, safety_m, salinity).mm
        });

    let mut warnings: Vec<String> = Vec::new();
    for (index, gas) in gas_forms.iter().enumerate() {
        let waypoint = index + 1;
        if !(1.0..=100.0).contains(&gas.o2) {
            warnings.push(i18n::t1(tr.warn_o2_range, waypoint));
        }
        if !(0.0..=100.0).contains(&gas.he) {
            warnings.push(i18n::t1(tr.warn_he_range, waypoint));
        }
        if gas.o2 + gas.he > 100.0 {
            warnings.push(i18n::t1(tr.warn_o2_he_sum, waypoint));
        }
    }
    if custom_value {
        for (index, form) in point_forms.iter().enumerate() {
            let waypoint = index + 1;
            if form.depth < 0.0 {
                warnings.push(i18n::t1(tr.warn_neg_depth, waypoint));
            }
            if form.minutes < 0.0 {
                warnings.push(i18n::t1(tr.warn_neg_duration, waypoint));
            }
        }
    }
    let gas_low = plan_gas_low(&plan, reserve_value);

    let graph_samples = plan.graph_samples(SURFACE_PRESSURE_MBAR / 1000.0, salinity, rmv_value);
    let pressure_colors: Vec<String> = plan
        .gas_uses
        .iter()
        .map(|gas| gas_color(&gas_forms, gas.gas).to_string())
        .collect();
    let graph_dive = Dive {
        max_depth: Some(max_depth),
        duration: Some(plan.total_time()),
        computers: vec![DiveComputer {
            model: "Planner".to_string(),
            samples: graph_samples,
            events: plan.events(),
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
        state.close_dialog();
    };

    let optimize = move |_| {
        let supplies: Vec<GasSupply> = (gases)()
            .iter()
            .filter(|gas| gas.enabled)
            .map(GasForm::supply)
            .collect();
        let time = DivePlan::max_bottom_time_for_reserve(
            prefs.depth_from_value((bottom_depth)()),
            &supplies,
            (max_ppo2)(),
            prefs.depth_from_value((switch_safety)()),
            Pressure::from_bar((reserve)()),
            if (deco_model_index)() == 1 {
                DecoModel::Vpmb {
                    conservatism: (vpmb_conservatism)(),
                }
            } else {
                DecoModel::Buhlmann {
                    gf_low: (gf_low)(),
                    gf_high: (gf_high)(),
                }
            },
            SURFACE_PRESSURE_MBAR / 1000.0,
            salinity,
            (rmv)(),
        );
        custom.set(false);
        bottom_minutes.set(time.seconds as f64 / 60.0);
    };

    let toggle_custom = move |_| {
        let next = !(custom)();
        if next {
            let depth = (bottom_depth)();
            let minutes = (bottom_minutes)();
            let descent = (prefs.depth_from_value(depth).meters() / 20.0 * 60.0).round();
            points.set(vec![
                PointForm {
                    depth,
                    minutes: descent,
                },
                PointForm { depth, minutes },
            ]);
            let mut set = expanded.write();
            set.clear();
            set.insert(0);
        }
        custom.set(next);
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
                            onclick: move |_| state.close_dialog(),
                            "\u{2039} {tr.back}"
                        }
                        h1 { "{tr.dive_planner}" }
                        div { class: "detail-actions",
                            button { class: "btn primary", onclick: on_save, "{tr.planner_save_dive}" }
                            button {
                                class: "btn",
                                onclick: move |_| state.close_dialog(),
                                "{tr.close}"
                            }
                        }
                    }
                    p { class: "muted", "{header_summary}" }
                    p { class: "muted planner-gauge-hint", "{tr.gauge_hint}" }
                }

                div { class: "section-title", "{tr.section_model_mode}" }
                div { class: "edit-form",
                        div { class: "planner-inline-fields",
                            label { class: "field-label", "{tr.dive_mode}"
                                select {
                                    class: "field",
                                    value: "{mode_value}",
                                    onchange: move |evt| mode_index.set(evt.value().parse().unwrap_or(0)),
                                    option { value: "0", "{tr.oc_label}" }
                                    option { value: "1", "{tr.ccr_label}" }
                                    option { value: "2", "{tr.pscr_label}" }
                                }
                            }
                            div { class: "field-label",
                                span { class: "label-row",
                                    "{tr.deco_model}"
                                    InfoTip { text: tr.tip_deco_model }
                                }
                                div {
                                    class: "planner-switch",
                                    role: "group",
                                    aria_label: "{tr.deco_model}",
                                    button {
                                        r#type: "button",
                                        class: if deco_model_value == 0 { "active" } else { "" },
                                        aria_pressed: if deco_model_value == 0 { "true" } else { "false" },
                                        onclick: move |_| deco_model_index.set(0),
                                        "B\u{fc}hlmann (GF)"
                                    }
                                    button {
                                        r#type: "button",
                                        class: if deco_model_value == 1 { "active" } else { "" },
                                        aria_pressed: if deco_model_value == 1 { "true" } else { "false" },
                                        onclick: move |_| deco_model_index.set(1),
                                        "VPM-B"
                                    }
                                }
                            }
                            if deco_model_value == 1 {
                                Gauge {
                                    label: tr.conservatism,
                                    unit: "",
                                    hint: tr.tip_conservatism,
                                    value: vpmb_conservatism_value as f64,
                                    min: 0.0,
                                    max: 4.0,
                                    step: 1.0,
                                    onchange: move |value: f64| vpmb_conservatism.set(value.round().clamp(0.0, 4.0) as u8),
                                }
                            } else {
                                Gauge {
                                    label: tr.gf_low,
                                    unit: "%",
                                    hint: tr.tip_gf_low,
                                    value: gf_low_value * 100.0,
                                    min: 10.0,
                                    max: 100.0,
                                    step: 1.0,
                                    onchange: move |value: f64| gf_low.set((value / 100.0).clamp(0.1, 1.0)),
                                }
                                Gauge {
                                    label: tr.gf_high,
                                    unit: "%",
                                    hint: tr.tip_gf_high,
                                    value: gf_high_value * 100.0,
                                    min: 10.0,
                                    max: 100.0,
                                    step: 1.0,
                                    onchange: move |value: f64| gf_high.set((value / 100.0).clamp(0.1, 1.0)),
                                }
                            }
                            if mode_value == 1 {
                                Gauge {
                                    label: tr.setpoint,
                                    unit: "bar",
                                    hint: tr.gauge_hint,
                                    value: setpoint_value,
                                    min: 0.5,
                                    max: 1.6,
                                    step: 0.1,
                                    decimals: 1,
                                    onchange: move |value: f64| setpoint.set(value),
                                }
                            }
                            if mode_value == 2 {
                                Gauge {
                                    label: tr.dump_ratio,
                                    unit: "%",
                                    hint: tr.gauge_hint,
                                    value: dump_ratio_value,
                                    min: 10.0,
                                    max: 200.0,
                                    step: 10.0,
                                    onchange: move |value: f64| dump_ratio.set(value),
                                }
                            }
                        }
                    }

                div { class: "section-title", "{tr.gas_cylinders}" }
                div { class: "planner-gases",
                    for (index, gas) in gas_forms.iter().enumerate() {
                        div {
                            key: "gas-{index}",
                            class: if gas.enabled { "gas-card" } else { "gas-card hidden-gas" },
                            div { class: "gas-card-head",
                                span {
                                    class: "gas-color-dot",
                                    style: "background: {gas_color(&gas_forms, gas.gas())};",
                                }
                                strong { "{gas.gas().name()}" }
                                span { class: "muted",
                                    "{tr.switch_depth} {prefs.depth(gas.switch_depth(max_ppo2_value, safety_m, salinity))}"
                                }
                                button {
                                    class: "btn gas-toggle",
                                    r#type: "button",
                                    title: if gas.enabled { tr.hide_gas } else { tr.show_gas },
                                    disabled: gas.enabled && switch_locked,
                                    onclick: move |_| {
                                        let mut list = gases.write();
                                        list[index].enabled = !list[index].enabled;
                                    },
                                    if gas.enabled { "{tr.hide_gas}" } else { "{tr.show_gas}" }
                                }
                                button {
                                    class: "icon-btn",
                                    title: "{tr.remove_gas}",
                                    disabled: gas_forms.len() <= 1,
                                    onclick: move |_| {
                                        gases.write().remove(index);
                                    },
                                    "\u{00d7}"
                                }
                            }
                            div { class: "gas-gauges",
                                Gauge {
                                    label: tr.tank_volume,
                                    unit: "L",
                                    hint: tr.gauge_hint,
                                    value: gas.size,
                                    min: 1.0,
                                    max: 38.0,
                                    step: 1.0,
                                    onchange: move |value| gases.write()[index].size = value,
                                }
                                Gauge {
                                    label: tr.pressure,
                                    unit: "bar",
                                    hint: tr.gauge_hint,
                                    value: gas.pressure,
                                    min: 0.0,
                                    max: 300.0,
                                    step: 10.0,
                                    onchange: move |value| gases.write()[index].pressure = value,
                                }
                                Gauge {
                                    label: "O2",
                                    unit: "%",
                                    hint: tr.gauge_hint,
                                    value: gas.o2,
                                    min: 21.0,
                                    max: 100.0,
                                    step: 1.0,
                                    onchange: move |value: f64| {
                                        let mut list = gases.write();
                                        let gas = &mut list[index];
                                        gas.o2 = value;
                                        // Keep O2 + He <= 100: adding oxygen
                                        // squeezes out helium automatically.
                                        gas.he = gas.he.min((100.0 - gas.o2).max(0.0));
                                    },
                                }
                                Gauge {
                                    label: "He",
                                    unit: "%",
                                    hint: tr.gauge_hint,
                                    value: gas.he,
                                    min: 0.0,
                                    max: (100.0 - gas.o2).max(0.0),
                                    step: 1.0,
                                    onchange: move |value: f64| {
                                        let mut list = gases.write();
                                        let gas = &mut list[index];
                                        gas.he = value.min((100.0 - gas.o2).max(0.0));
                                    },
                                }
                            }
                        }
                    }
                }
                button {
                    class: "btn",
                    onclick: move |_| gases.write().push(GasForm::default()),
                    "{tr.add_gas}"
                }

                div { class: "section-title", "{tr.user_parameters}" }
                div { class: "planner-gauges-row",
                    Gauge {
                        label: tr.rmv,
                        unit: "L/min",
                        hint: tr.gauge_hint,
                        value: rmv_value,
                        min: 5.0,
                        max: 60.0,
                        step: 1.0,
                        onchange: move |value| rmv.set(value),
                    }
                    Gauge {
                        label: tr.max_ppo2,
                        unit: "bar",
                        hint: tr.gauge_hint,
                        value: max_ppo2_value,
                        min: 1.4,
                        max: 2.0,
                        step: 0.1,
                        decimals: 1,
                        onchange: move |value| max_ppo2.set(value),
                    }
                    Gauge {
                        label: tr.switch_safety,
                        unit: prefs.depth_unit(),
                        hint: tr.gauge_hint,
                        value: switch_safety(),
                        min: 0.0,
                        max: max_safety_gauge,
                        step: 0.5,
                        decimals: 1,
                        disabled: switch_locked,
                        onchange: move |value| switch_safety.set(value),
                    }
                }

                div { class: "section-title", "{tr.dive_planning}" }
                if !custom_value {
                    div { class: "dive-gauges",
                        Gauge {
                            label: tr.bottom_depth,
                            unit: prefs.depth_unit(),
                            hint: tr.gauge_hint,
                            value: depth_value,
                            min: 0.0,
                            max: max_depth_gauge,
                            step: 1.0,
                            big: true,
                            onchange: move |value| bottom_depth.set(value),
                        }
                        Gauge {
                            label: tr.bottom_minutes,
                            unit: "min",
                            hint: tr.gauge_hint,
                            value: minutes_value,
                            min: 0.0,
                            max: 180.0,
                            step: 1.0,
                            big: true,
                            onchange: move |value| bottom_minutes.set(value),
                        }
                        Gauge {
                            label: tr.reserve_pressure,
                            unit: "bar",
                            hint: tr.gauge_hint,
                            value: reserve(),
                            min: 0.0,
                            max: 200.0,
                            step: 10.0,
                            onchange: move |value| reserve.set(value),
                        }
                    }
                }

                div { class: "planner-actions",
                    if !custom_value {
                        button {
                            class: "btn",
                            onclick: optimize,
                            "{tr.optimize_time}"
                        }
                    }
                    button {
                        class: "btn",
                        onclick: toggle_custom,
                        if custom_value {
                            "{tr.square_profile}"
                        } else {
                            "{tr.customize_waypoints}"
                        }
                    }
                }

                if custom_value {
                    div { class: "plan-points",
                        {point_forms.iter().enumerate().map(|(index, form)| {
                            let is_open = (expanded)().contains(&index);
                            let form = form.clone();
                            let run_seconds: i32 = point_forms.iter().take(index + 1).map(|p| p.minutes.round() as i32 * 60).sum();
                            let run = format_duration(Duration::new(run_seconds));
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
                                        }
                                        button {
                                            class: "icon-btn point-remove",
                                            title: "{tr.remove_waypoint}",
                                            disabled: point_forms.len() <= 1,
                                            onclick: move |_| { points.write().remove(index); },
                                            "\u{00d7}"
                                        }
                                    }
                                    if is_open {
                                        div { class: "plan-point-body",
                                            Gauge {
                                                label: tr.bottom_depth,
                                                unit: prefs.depth_unit(),
                                                hint: tr.gauge_hint,
                                                value: form.depth,
                                                min: 0.0,
                                                max: max_depth_gauge,
                                                step: 1.0,
                                                onchange: move |value| points.write()[index].depth = value,
                                            }
                                            Gauge {
                                                label: tr.bottom_minutes,
                                                unit: "min",
                                                hint: tr.gauge_hint,
                                                value: form.minutes,
                                                min: 0.0,
                                                max: 180.0,
                                                step: 1.0,
                                                onchange: move |value| points.write()[index].minutes = value,
                                            }
                                            label { class: "field-label", "{tr.run_time}"
                                                div { class: "readout", "{run}" }
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
                            let mut list = points.write();
                            let next = list
                                .last()
                                .cloned()
                                .map(|mut point| {
                                    point.minutes = 0.0;
                                    point
                                })
                                .unwrap_or_default();
                            list.push(next);
                        },
                        "{tr.add_waypoint}"
                    }
                }


                div { class: "section-title", "{tr.profile}" }
                DiveProfile {
                    dive: graph_dive,
                    dc_index: 0,
                    show_all: true,
                    pressure_colors,
                    planned: true,
                }

                if over_mod {
                    p { class: "warn", "{tr.warn_over_mod}" }
                }
                if gas_low {
                    p { class: "warn", "{tr.gas_not_enough}" }
                }
                for warning in warnings.iter() {
                    p { class: "warn", "{warning}" }
                }

                PlanResults {
                    plan: plan.clone(),
                    gases: gas_forms.clone(),
                    bottom_gas,
                    reserve: reserve_value,
                    ndl: ndl(),
                    mod_mm,
                    end_mm,
                    rmv: rmv_value,
                    salinity,
                }
            }
        }
    }
}

/// Whether the last bottle of any gas mix ends below the reserve. A partly
/// used first bottle may be empty by design when a second bottle of the same
/// gas takes over, so only the last bottle of each mix counts.
fn plan_gas_low(plan: &DivePlan, reserve: Pressure) -> bool {
    low_bottles(plan, reserve).into_iter().any(|low| low)
}

/// For each bottle in `plan.gas_uses`, whether it ends below the reserve while
/// being the last bottle of its gas mix.
fn low_bottles(plan: &DivePlan, reserve: Pressure) -> Vec<bool> {
    let mut last_for_gas = std::collections::HashMap::new();
    for (index, gas) in plan.gas_uses.iter().enumerate() {
        last_for_gas.insert((gas.gas.o2_permille, gas.gas.he_permille), index);
    }
    plan.gas_uses
        .iter()
        .enumerate()
        .map(|(index, gas)| {
            last_for_gas[&(gas.gas.o2_permille, gas.gas.he_permille)] == index
                && gas.end_pressure.mbar < reserve.mbar
        })
        .collect()
}

/// The summary facts and the gas, switch and stop tables of a plan.
#[allow(clippy::too_many_arguments)]
#[component]
fn PlanResults(
    plan: DivePlan,
    gases: Vec<GasForm>,
    bottom_gas: GasMix,
    reserve: Pressure,
    ndl: Option<Duration>,
    mod_mm: i32,
    end_mm: i32,
    rmv: f64,
    salinity: i32,
) -> Element {
    let state = use_context::<AppState>();
    let prefs = (state.prefs)();
    let tr = state.strings();
    let max_depth = plan.max_depth();
    let rebreather = plan.bottom_mode().is_rebreather();
    let ndl_text = match ndl {
        Some(d) if d.seconds < 24 * 3600 => format_duration(d),
        _ => "> 24 h".to_string(),
    };
    let stops: Vec<(String, String)> = plan
        .stops
        .iter()
        .map(|stop| (prefs.depth(stop.depth), format_duration(stop.duration)))
        .collect();
    let switches: Vec<(String, String, String, String)> = plan
        .gas_switches
        .iter()
        .map(|switch| {
            (
                format_duration(switch.time),
                prefs.depth(switch.depth),
                switch.gas.name(),
                gas_color(&gases, switch.gas).to_string(),
            )
        })
        .collect();
    let gas_usage: Vec<(String, f64, f64, f64, f64, bool, String)> = plan
        .gas_uses
        .iter()
        .zip(low_bottles(&plan, reserve))
        .map(|(gas, low)| {
            (
                gas.gas.name(),
                gas.size.liters(),
                gas.start_pressure.bar(),
                gas.used_liters,
                gas.end_pressure.bar(),
                low,
                gas_color(&gases, gas.gas).to_string(),
            )
        })
        .collect();
    let max_tts = plan.max_tts();
    let max_tts_current_gas = plan.max_tts_current_gas(SURFACE_PRESSURE_MBAR / 1000.0, salinity);
    let has_stops = !stops.is_empty();
    let gas_needed: f64 = plan.gas_uses.iter().map(|gas| gas.used_liters).sum();

    rsx! {
            div { class: "section-title", "{tr.summary}" }
            div { class: "facts planner-results",
                Result { label: tr.bottom_gas, value: bottom_gas.name() }
                Result { label: tr.ambient, value: format!("{:.2} bar", ambient_mbar(max_depth.mm, SURFACE_PRESSURE_MBAR, salinity) / 1000.0) }
                Result {
                    label: tr.mod_depth,
                    value: prefs.depth(Depth::new(mod_mm)),
                    help: Some(tr.tip_mod),
                }
                if prefs.depth(Depth::new(end_mm)) != prefs.depth(max_depth) {
                    Result {
                        label: tr.end,
                        value: prefs.depth(Depth::new(end_mm)),
                        help: Some(tr.tip_end),
                    }
                }
                Result {
                    label: tr.ndl,
                    value: ndl_text,
                    help: Some(tr.tip_ndl),
                }
                Result { label: tr.runtime, value: format_duration(plan.total_time()) }
                Result { label: tr.bottom_time, value: format_duration(plan.bottom_time()) }
                Result { label: tr.deco_time, value: format_duration(plan.deco_time()) }
                Result {
                    label: tr.max_tts,
                    value: format_duration(max_tts),
                    help: Some(tr.tip_max_tts),
                }
                if max_tts_current_gas != max_tts {
                    Result {
                        label: tr.max_tts_current_gas,
                        value: format_duration(max_tts_current_gas),
                        help: Some(tr.tip_max_tts_current_gas),
                    }
                }
                if !rebreather {
                    Result {
                        label: tr.gas_needed,
                        value: format!("{gas_needed:.0} L"),
                        help: Some(tr.tip_gas_needed),
                    }
                } else {
                    Result {
                        label: tr.oc_bailout,
                        value: format!("{:.0} L", plan.bailout_liters(rmv, SURFACE_PRESSURE_MBAR / 1000.0, salinity)),
                        help: Some(tr.tip_bailout),
                    }
                }
            }

            if !gas_usage.is_empty() {
                div { class: "section-title", "{tr.gas_usage}" }
                div { class: "table-scroll",
                    table { class: "data-table",
                        thead { tr { th { "{tr.gas_gas}" } th { "{tr.tank_size}" } th { "{tr.start_pressure}" } th { "{tr.used_gas}" } th { "{tr.gas_left}" } } }
                        tbody {
                            for (name, size, start, used, left, low, color) in gas_usage.iter() {
                                tr {
                                    td {
                                        span {
                                            class: "gas-color-dot",
                                            style: "background: {color};",
                                        }
                                        "{name}"
                                    }
                                    td { "{size:.1} L" }
                                    td { "{start:.0} bar" }
                                    td { "{used:.0} L" }
                                    td { class: if *low { "warn" } else { "" }, "{left:.0} bar" }
                                }
                            }
                        }
                    }
                }
            }

            if !switches.is_empty() {
                div { class: "section-title", "{tr.gas_switches}" }
                table { class: "data-table",
                    thead { tr { th { "{tr.time}" } th { "{tr.stop}" } th { "{tr.gas_gas}" } } }
                    tbody {
                        for (time, depth, gas, color) in switches.iter() {
                            tr {
                                td { "{time}" }
                                td { "{depth}" }
                                td {
                                    span {
                                        class: "gas-color-dot",
                                        style: "background: {color};",
                                    }
                                    "{gas}"
                                }
                            }
                        }
                    }
                }
            }

            if has_stops {
                div { class: "section-title", "{tr.deco_schedule}" }
                table { class: "data-table",
                    thead { tr { th { "{tr.stop}" } th { "{tr.time}" } } }
                    tbody {
                        for (stop_depth, stop_time) in stops.iter() {
                            tr { td { "{stop_depth}" } td { "{stop_time}" } }
                        }
                    }
                }
            } else {
                p { class: "muted", "{tr.no_stops}" }
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
