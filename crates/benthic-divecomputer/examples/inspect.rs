//! Developer aid: print a summary of each bundled libdivecomputer fixture.

use benthic_divecomputer::parse_dump;

fn main() {
    let base = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../vendor/libdivecomputer/test/fixtures"
    );
    let fixtures = [
        ("Shearwater", "Petrel 2", "shearwater_petrel2-0001.bin"),
        ("Heinrichs Weikamp", "OSTC 5", "hw_ostc5-0001.bin"),
        ("Garmin", "Descent™ Mk1", "garmin_descent_mk1-0001.bin"),
    ];
    for (vendor, product, file) in fixtures {
        let data = std::fs::read(format!("{base}/{file}")).unwrap();
        let dive = parse_dump(vendor, product, &data).unwrap();
        let computer = &dive.computers[0];
        println!("=== {vendor} {product} ({file}) ===");
        println!(
            "when={} duration={:?} max={:?} mean={:?} mode={:?} serial={:?} fw={:?}",
            dive.when,
            computer.duration,
            computer.max_depth,
            computer.mean_depth,
            computer.divemode,
            computer.serial,
            computer.firmware
        );
        println!(
            "samples={} events={} cylinders={}",
            computer.samples.len(),
            computer.events.len(),
            dive.cylinders.len()
        );
        for cylinder in &dive.cylinders {
            println!("  cylinder {cylinder:?}");
        }
        let count =
            |f: fn(&benthic_core::Sample) -> bool| computer.samples.iter().filter(|s| f(s)).count();
        println!(
            "  pressure={} temp={} setpoint={} o2={} ndl={} cns={}",
            count(|s| !s.pressures.is_empty()),
            count(|s| s.temperature.is_some()),
            count(|s| s.setpoint.is_some()),
            count(|s| !s.o2_sensors.is_empty()),
            count(|s| s.ndl.is_some()),
            count(|s| s.cns.is_some()),
        );
        for event in computer.events.iter().take(8) {
            println!("  event {:?}", event);
        }
        println!("  last {:?}", computer.samples.last());
    }
}
