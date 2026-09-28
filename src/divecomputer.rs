//! Shared dive-computer helpers used by the native and web download dialogs.
#![cfg(any(feature = "divecomputer", target_arch = "wasm32"))]

use std::collections::BTreeMap;

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
