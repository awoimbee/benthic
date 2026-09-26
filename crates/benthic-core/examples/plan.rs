//! Developer utility: print the NDL and a Bühlmann schedule for a dive.
//!
//! Usage: `cargo run -p benthic-core --example plan -- [depth_m] [minutes]
//!         [gf_low] [gf_high] [salinity] [o2_percent] [he_percent]`

use benthic_core::deco::{BreathingMode, Buhlmann, DecoSegment};
use benthic_core::gas::{GasMix, SURFACE_PRESSURE_MBAR};
use benthic_core::units::{format_depth_m, format_duration, Depth, Duration};

fn main() {
    let mut args = std::env::args().skip(1);
    let depth_m: f64 = args.next().and_then(|a| a.parse().ok()).unwrap_or(40.0);
    let minutes: f64 = args.next().and_then(|a| a.parse().ok()).unwrap_or(40.0);
    let gf_low: f64 = args.next().and_then(|a| a.parse().ok()).unwrap_or(1.0);
    let gf_high: f64 = args.next().and_then(|a| a.parse().ok()).unwrap_or(1.0);
    let salinity: i32 = args.next().and_then(|a| a.parse().ok()).unwrap_or(10_300);
    let o2: f64 = args.next().and_then(|a| a.parse().ok()).unwrap_or(21.0);
    let he: f64 = args.next().and_then(|a| a.parse().ok()).unwrap_or(0.0);

    let gas = GasMix::new((o2 * 10.0).round() as u16, (he * 10.0).round() as u16);
    let depth = Depth::from_meters(depth_m);
    let model = Buhlmann::new(SURFACE_PRESSURE_MBAR / 1000.0, salinity);

    let ndl = model
        .ndl(depth, gas, 1.0)
        .map(format_duration)
        .unwrap_or_else(|| "> 24 h".to_string());
    println!("NDL (GF 100/100) at {depth_m} m on {o2:.0}/{he:.0}: {ndl}");

    let plan = DecoSegment {
        bottom_depth: depth,
        bottom_minutes: minutes,
        mode: BreathingMode::OpenCircuit(gas),
        gf_low,
        gf_high,
        ..Default::default()
    };
    let tissues = model.tissues_at_ascent(&plan);
    let first_ceiling = model.gf_ceiling_depth_of(
        &tissues,
        gf_low,
        gf_high,
        model.gf_anchor_bar(&tissues, gf_low),
    );
    println!("first ceiling at start of ascent: {first_ceiling:?}");

    let stops = model.deco_schedule(&plan);
    println!(
        "\nBühlmann GF {:.0}/{:.0}, {depth_m} m for {minutes} min on {o2:.0}/{he:.0}:",
        gf_low * 100.0,
        gf_high * 100.0
    );
    if stops.is_empty() {
        println!("  no deco");
    }
    for stop in &stops {
        println!(
            "  {} for {}",
            format_depth_m(stop.depth),
            format_duration(stop.duration)
        );
    }
    let total: i32 = stops.iter().map(|s| s.duration.seconds).sum();
    println!("  total deco {}", format_duration(Duration::new(total)));
}
