//! Offline coverage: parse the raw dumps that ship with libdivecomputer.
//!
//! `vendor/libdivecomputer/test/fixtures/manifest.txt` records the model each
//! blob was recorded with, so every fixture is a real end-to-end parse.

use std::path::PathBuf;

use benthic_core::Divemode;
use benthic_divecomputer::{descriptors, parse_dump};

fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/libdivecomputer/test/fixtures")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

#[test]
fn lists_supported_devices() {
    let all = descriptors().expect("enumerate descriptors");
    assert!(
        all.len() > 100,
        "expected a large device table, got {}",
        all.len()
    );
    assert!(all
        .iter()
        .any(|d| d.vendor == "Shearwater" && d.product == "Petrel 2"));
    assert!(all.iter().any(|d| d.vendor == "Heinrichs Weikamp"));
    assert!(all
        .iter()
        .any(|d| d.vendor == "Garmin" && d.product.contains("Mk1")));
    // Every descriptor should advertise at least one transport.
    assert!(all.iter().all(|d| !d.transport_list().is_empty()));
}

fn assert_parses(vendor: &str, product: &str, file: &str) -> benthic_core::Dive {
    let data = fixture(file);
    let dive = parse_dump(vendor, product, &data)
        .unwrap_or_else(|e| panic!("parse {file} as {vendor} {product}: {e}"));

    assert_eq!(dive.computers.len(), 1, "{file}: one computer");
    let computer = &dive.computers[0];
    assert!(!computer.samples.is_empty(), "{file}: samples");
    assert!(
        computer.duration.is_some_and(|d| d.seconds > 0),
        "{file}: duration"
    );
    assert!(
        computer.max_depth.is_some_and(|d| d.mm > 0),
        "{file}: max depth"
    );
    assert!(dive.when > 0, "{file}: start time");
    assert!(
        computer.samples.iter().any(|s| s.depth.mm > 0),
        "{file}: a sample below the surface"
    );
    dive
}

#[test]
fn parses_shearwater_petrel2() {
    let dive = assert_parses("Shearwater", "Petrel 2", "shearwater_petrel2-0001.bin");
    let computer = &dive.computers[0];
    assert!(computer.samples.len() > 100, "dense profile");
    assert_eq!(computer.divemode, Divemode::Ccr);
    // Two gas mixes are reported; there is no tank pressure in this dump.
    assert_eq!(dive.cylinders.len(), 2);
    assert!(dive.cylinders.iter().any(|c| c.gas.o2_permille == 320));
    assert!(computer.samples.iter().any(|s| s.setpoint.is_some()));
    assert!(computer.samples.iter().any(|s| s.cns.is_some()));
    assert!(computer.samples.iter().any(|s| s.temperature.is_some()));
    assert!(computer.events.iter().any(|e| e.name == "gaschange"));
}

#[test]
fn parses_heinrichs_weikamp_ostc5() {
    let dive = assert_parses("Heinrichs Weikamp", "OSTC 5", "hw_ostc5-0001.bin");
    let computer = &dive.computers[0];
    assert!(computer.samples.len() > 1000);
    assert_eq!(computer.divemode, Divemode::Ccr);
    assert!(computer.mean_depth.is_some());
    assert!(computer.samples.iter().any(|s| s.setpoint.is_some()));
}

#[test]
fn parses_garmin_descent_mk1() {
    let dive = assert_parses("Garmin", "Descent™ Mk1", "garmin_descent_mk1-0001.bin");
    let computer = &dive.computers[0];
    assert_eq!(computer.divemode, Divemode::OpenCircuit);
    assert!(computer.samples.len() > 100);
    assert_eq!(computer.serial.as_deref(), Some("3991472814"));
    assert!(computer.firmware.is_some());
}

#[test]
fn rejects_unknown_device() {
    let data = fixture("hw_ostc5-0001.bin");
    assert!(parse_dump("Nonexistent", "Computer", &data).is_err());
}
