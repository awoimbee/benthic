//! Developer utility: parse a dive log and print a short summary.
//!
//! Usage: `cargo run -p benthic-core --example parse_ssrf -- <path>`

use std::path::PathBuf;

fn main() {
    let path: PathBuf = std::env::args()
        .nth(1)
        .expect("usage: parse_ssrf <path>")
        .into();

    let text = std::fs::read_to_string(&path).expect("read file");

    match benthic_core::io::parse_auto(&text) {
        Ok(log) => {
            println!(
                "Parsed {} dives, {} trips, {} sites, {} devices",
                log.dives.len(),
                log.trips.len(),
                log.sites.len(),
                log.devices.len()
            );
            println!(
                "First dive: {}",
                benthic_core::units::format_timestamp_utc(
                    log.dives_sorted().first().map(|d| d.when).unwrap_or(0)
                )
            );
            if let Some(dive) = log.dives_sorted().first() {
                let samples = dive
                    .primary_computer()
                    .map(|dc| dc.samples.len())
                    .unwrap_or(0);
                let events = dive
                    .primary_computer()
                    .map(|dc| dc.events.len())
                    .unwrap_or(0);
                println!(
                    "  number={} duration={:?} max_depth={:?} cylinders={} samples={} events={} tags={:?}",
                    dive.number,
                    dive.duration(),
                    dive.max_depth(),
                    dive.cylinders.len(),
                    samples,
                    events,
                    dive.tags
                );
            }

            // Round-trip through the writer and re-parse to check consistency.
            let out = benthic_core::io::ssrf::write_string(&log);
            match benthic_core::io::ssrf::parse_str(&out) {
                Ok(again) => println!(
                    "Round-trip OK: {} dives, {} samples on first",
                    again.dives.len(),
                    again
                        .dives
                        .first()
                        .and_then(|d| d.primary_computer())
                        .map(|dc| dc.samples.len())
                        .unwrap_or(0)
                ),
                Err(e) => {
                    println!("Round-trip FAILED: {e}");
                    std::process::exit(1);
                }
            }
        }
        Err(e) => {
            eprintln!("Parse error: {e}");
            std::process::exit(1);
        }
    }
}
