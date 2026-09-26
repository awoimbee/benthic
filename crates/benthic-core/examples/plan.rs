//! Developer utility: print the NDL and a Bühlmann schedule for a dive.
//!
//! Usage: `cargo run -p benthic-core --example plan -- [depth_m] [minutes] [gf_low] [gf_high]`

use benthic_core::deco::{BreathingMode, Buhlmann, DecoSegment};
use benthic_core::gas::AIR;
use benthic_core::units::{format_depth_m, format_duration, Depth, Duration};

fn main() {
    let mut args = std::env::args().skip(1);
    let depth_m: f64 = args.next().and_then(|a| a.parse().ok()).unwrap_or(40.0);
    let minutes: f64 = args.next().and_then(|a| a.parse().ok()).unwrap_or(40.0);
    let gf_low: f64 = args.next().and_then(|a| a.parse().ok()).unwrap_or(1.0);
    let gf_high: f64 = args.next().and_then(|a| a.parse().ok()).unwrap_or(1.0);

    let depth = Depth::from_meters(depth_m);
    let model = Buhlmann::default();

    let ndl = model
        .ndl(depth, AIR, 1.0)
        .map(format_duration)
        .unwrap_or_else(|| "> 24 h".to_string());
    println!("NDL (GF 100/100) at {depth_m} m on air: {ndl}");

    let stops = model.deco_schedule(&DecoSegment {
        bottom_depth: depth,
        bottom_minutes: minutes,
        mode: BreathingMode::OpenCircuit(AIR),
        gf_low,
        gf_high,
        ..Default::default()
    });

    println!(
        "\nBühlmann GF {:.0}/{:.0}, {depth_m} m for {minutes} min on air:",
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
