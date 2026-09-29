//! The web (wasm) dive-computer backend.
//!
//! Bridges to the Emscripten shim through `globalThis.benthicWeb`, which is
//! installed on demand by `public/divecomputer/api.js`. Everything crosses the
//! boundary as JSON, so the Rust side reuses `benthic_core`'s shared mapping.
#![cfg(target_arch = "wasm32")]

use js_sys::{Array, Function, Promise, Reflect};
use serde::Deserialize;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

use benthic_core::divecomputer::RawDive;

/// A model from the shim's descriptor table.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct WebModel {
    pub vendor: String,
    pub product: String,
    /// libdivecomputer transport bitmask.
    pub transports: u32,
}

impl WebModel {
    pub fn name(&self) -> String {
        format!("{} {}", self.vendor, self.product)
    }
}

/// The result of a download, as produced by `benthic_dc_download`.
#[derive(Debug, Deserialize)]
pub struct WebDownload {
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub fingerprint: String,
    #[serde(default)]
    pub dives: Vec<RawDive>,
}

fn window() -> Result<JsValue, String> {
    web_sys::window()
        .map(JsValue::from)
        .ok_or_else(|| "no browser window".to_string())
}

fn api() -> Result<JsValue, String> {
    let window = window()?;
    let api = Reflect::get(&window, &JsValue::from_str("benthicWeb"))
        .map_err(|_| "web dive-computer API is not loaded".to_string())?;
    if api.is_undefined() || api.is_null() {
        Err("web dive-computer API is not loaded".to_string())
    } else {
        Ok(api)
    }
}

/// Loads `public/divecomputer/api.js` on demand.
pub async fn ensure_loaded() -> Result<(), String> {
    if api().is_ok() {
        return Ok(());
    }
    let window = window()?;
    let loader = Reflect::get(&window, &JsValue::from_str("benthicLoadDc"))
        .map_err(|_| "dive-computer loader is not installed".to_string())?;
    let loader: Function = loader
        .dyn_into()
        .map_err(|_| "dive-computer loader is not a function".to_string())?;
    let promise: Promise = loader
        .call0(&window)
        .map_err(|error| format!("{error:?}"))?
        .dyn_into()
        .map_err(|_| "dive-computer loader did not return a promise".to_string())?;
    JsFuture::from(promise)
        .await
        .map_err(|error| format!("{error:?}"))?;
    Ok(())
}

async fn call(method: &str, args: &[JsValue]) -> Result<JsValue, String> {
    let api = api()?;
    let function = Reflect::get(&api, &JsValue::from_str(method))
        .map_err(|_| format!("{method} is missing"))?;
    let function: Function = function
        .dyn_into()
        .map_err(|_| format!("{method} is not a function"))?;
    let arguments = Array::new();
    for argument in args {
        arguments.push(argument);
    }
    let promise: Promise = function
        .apply(&api, &arguments)
        .map_err(|error| format!("{error:?}"))?
        .dyn_into()
        .map_err(|_| format!("{method} did not return a promise"))?;
    JsFuture::from(promise)
        .await
        .map_err(|error| format!("{error:?}"))
}

async fn call_string(method: &str, args: &[JsValue]) -> Result<String, String> {
    call(method, args)
        .await?
        .as_string()
        .ok_or_else(|| format!("{method} did not return a string"))
}

/// Whether the browser exposes WebSerial.
pub async fn supported() -> bool {
    if ensure_loaded().await.is_err() {
        return false;
    }
    let Ok(api) = api() else { return false };
    let Ok(function) = Reflect::get(&api, &JsValue::from_str("supported")) else {
        return false;
    };
    let Ok(function): Result<Function, _> = function.dyn_into() else {
        return false;
    };
    function
        .call0(&api)
        .map(|value| value.is_truthy())
        .unwrap_or(false)
}

/// Enumerate every model libdivecomputer supports.
pub async fn descriptors() -> Result<Vec<WebModel>, String> {
    ensure_loaded().await?;
    let text = call_string("descriptorsJson", &[]).await?;
    serde_json::from_str(&text).map_err(|error| error.to_string())
}

/// Whether the browser exposes Web Bluetooth.
pub async fn bluetooth_supported() -> bool {
    if ensure_loaded().await.is_err() {
        return false;
    }
    let Ok(api) = api() else { return false };
    let Ok(function) = Reflect::get(&api, &JsValue::from_str("bluetoothSupported")) else {
        return false;
    };
    let Ok(function): Result<Function, _> = function.dyn_into() else {
        return false;
    };
    function
        .call0(&api)
        .map(|value| value.is_truthy())
        .unwrap_or(false)
}

/// Prompt the user to choose a device (must run from a user gesture).
pub async fn connect(transport: u32) -> Result<bool, String> {
    ensure_loaded().await?;
    Ok(call("connect", &[JsValue::from_f64(transport as f64)])
        .await?
        .as_bool()
        .unwrap_or(false))
}

/// Download the unseen dives from the selected device.
pub async fn download(
    vendor: &str,
    product: &str,
    transport: u32,
    fingerprint_hex: &str,
) -> Result<WebDownload, String> {
    ensure_loaded().await?;
    let text = call_string(
        "downloadJson",
        &[
            JsValue::from_str(vendor),
            JsValue::from_str(product),
            JsValue::from_f64(transport as f64),
            JsValue::from_str(fingerprint_hex),
        ],
    )
    .await?;
    serde_json::from_str(&text).map_err(|error| error.to_string())
}

/// libdivecomputer's `DC_TRANSPORT_SERIAL` bit.
pub const TRANSPORT_SERIAL: u32 = 1 << 0;
/// libdivecomputer's `DC_TRANSPORT_BLE` bit.
pub const TRANSPORT_BLE: u32 = 1 << 5;
