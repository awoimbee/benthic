//! Shared dive-computer helpers used by the native and web download dialogs.
#![cfg(any(feature = "divecomputer", target_arch = "wasm32"))]

use std::collections::BTreeMap;

use benthic_core::Dive;

/// The key under which a device's download fingerprint is remembered. The
/// address distinguishes two computers of the same model.
pub fn fingerprint_key(vendor: &str, product: &str, address: &str) -> String {
    if address.is_empty() {
        format!("{vendor} {product}")
    } else {
        format!("{vendor} {product} @ {address}")
    }
}

fn fingerprints() -> Option<BTreeMap<String, String>> {
    crate::storage::load_device_fingerprints().and_then(|text| serde_json::from_str(&text).ok())
}

/// The last-seen fingerprint for a device, or empty to download everything.
pub fn load_fingerprint(key: &str) -> Vec<u8> {
    fingerprints()
        .and_then(|map| map.get(key).map(|hex| from_hex(hex)))
        .unwrap_or_default()
}

/// Remember the most recent fingerprint so the next download skips old dives.
pub fn save_fingerprint(key: &str, value: &[u8]) {
    let mut map = fingerprints().unwrap_or_default();
    map.insert(key.to_string(), to_hex(value));
    if let Ok(text) = serde_json::to_string(&map) {
        let _ = crate::storage::save_device_fingerprints(&text);
    }
}

pub fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

pub fn from_hex(text: &str) -> Vec<u8> {
    text.as_bytes()
        .chunks(2)
        .filter_map(|pair| {
            std::str::from_utf8(pair)
                .ok()
                .and_then(|pair| u8::from_str_radix(pair, 16).ok())
        })
        .collect()
}

/// Vendor/product pair for the built-in mock dive computer.
pub const MOCK_VENDOR: &str = "Mock";
pub const MOCK_PRODUCT: &str = "Dive Computer";

pub fn is_mock(vendor: &str, product: &str) -> bool {
    vendor == MOCK_VENDOR && product == MOCK_PRODUCT
}

/// Five synthetic dives dated to the current UTC day, for exercising the
/// download flow without hardware.
pub fn mock_dives() -> Vec<Dive> {
    use benthic_core::gas::AIR;
    use benthic_core::units::{Depth, Duration, Pressure, Temperature, Volume};
    use benthic_core::{Cylinder, CylinderUse, DiveComputer, Divemode, Sample, EN13319_SALINITY};

    let now = crate::platform::now_secs();
    let day_start = now - now.rem_euclid(86_400);

    (0..5)
        .map(|index| {
            let depth = Depth::from_meters(18.0 + index as f64 * 5.0);
            let bottom_minutes = 22 + index * 4;
            let descent_seconds = (depth.meters() / 18.0 * 60.0).round() as i32;
            let ascent_to_stop = ((depth.meters() - 5.0) / 9.0 * 60.0).round() as i32;
            let safety_seconds = 180;
            let final_ascent = (5.0f64 / 9.0 * 60.0).round() as i32;
            let bottom_seconds = bottom_minutes * 60;
            let mut time = 0;
            let mut samples = Vec::with_capacity(8);
            let mut push = |seconds: i32, depth: Depth| {
                samples.push(Sample {
                    time: Duration::new(seconds),
                    depth,
                    temperature: Some(Temperature::from_celsius(18.0)),
                    ..Default::default()
                });
            };
            push(0, Depth::ZERO);
            time += descent_seconds;
            push(time, depth);
            time += bottom_seconds;
            push(time, depth);
            time += ascent_to_stop;
            push(time, Depth::from_meters(5.0));
            time += safety_seconds;
            push(time, Depth::from_meters(5.0));
            time += final_ascent;
            push(time, Depth::ZERO);

            let duration = Duration::new(time);
            let when = day_start + 9 * 3600 + index as i64 * 2 * 3600;
            Dive {
                id: index as u32 + 1,
                when,
                duration: Some(duration),
                max_depth: Some(depth),
                water_temp: Some(Temperature::from_celsius(18.0)),
                air_temp: Some(Temperature::from_celsius(24.0)),
                salinity: Some(EN13319_SALINITY),
                cylinders: vec![Cylinder {
                    size: Some(Volume::from_liters(12.0)),
                    working_pressure: Some(Pressure::from_bar(200.0)),
                    description: "12 L steel".to_string(),
                    gas: AIR,
                    start_pressure: Some(Pressure::from_bar(200.0)),
                    end_pressure: Some(Pressure::from_bar(70.0)),
                    use_: CylinderUse::OcGas,
                    ..Default::default()
                }],
                computers: vec![DiveComputer {
                    model: format!("{MOCK_VENDOR} {MOCK_PRODUCT}"),
                    serial: Some(format!("MOCK-{}", index + 1)),
                    divemode: Divemode::OpenCircuit,
                    max_depth: Some(depth),
                    water_temp: Some(Temperature::from_celsius(18.0)),
                    duration: Some(duration),
                    samples,
                    ..Default::default()
                }],
                ..Default::default()
            }
        })
        .collect()
}
