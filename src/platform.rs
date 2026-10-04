//! Platform-specific helpers: file export and the current time.
//!
//! Reading imports is handled uniformly by Dioxus' file input + `FileData`,
//! so only "save to disk / trigger a download" (and clock access) needs
//! per-platform code.

/// Current time as Unix seconds.
#[cfg(target_arch = "wasm32")]
pub fn now_secs() -> i64 {
    (js_sys::Date::now() / 1000.0) as i64
}

/// Current time as Unix seconds.
#[cfg(not(target_arch = "wasm32"))]
pub fn now_secs() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Whether the browser exposes Web Serial or Web Bluetooth at all.
///
/// This is a cheap feature probe (`"serial" in navigator`) used to decide
/// whether to offer the dive-computer download button; it deliberately does
/// not load the heavy Emscripten shim, which stays on demand.
#[cfg(target_arch = "wasm32")]
pub fn web_transport_available() -> bool {
    use wasm_bindgen::JsValue;
    let Some(window) = web_sys::window() else {
        return false;
    };
    let navigator = window.navigator();
    let has = |name: &str| {
        js_sys::Reflect::get(navigator.as_ref(), &JsValue::from_str(name))
            .map(|value| !value.is_undefined() && !value.is_null())
            .unwrap_or(false)
    };
    has("serial") || has("bluetooth")
}

/// Ask the browser for the device's current position (latitude, longitude).
/// Returns `None` when permission is denied or geolocation is unavailable.
#[cfg(target_arch = "wasm32")]
pub async fn current_location() -> Option<(f64, f64)> {
    use wasm_bindgen::prelude::*;
    use wasm_bindgen_futures::JsFuture;

    let geolocation = web_sys::window()?.navigator().geolocation().ok()?;
    let promise = js_sys::Promise::new(&mut |resolve, reject| {
        let resolve_cb = Closure::once(move |position: JsValue| {
            let _ = resolve.call1(&JsValue::NULL, &position);
        });
        let reject_cb = Closure::once(move |error: JsValue| {
            let _ = reject.call1(&JsValue::NULL, &error);
        });
        let _ = geolocation.get_current_position_with_error_callback(
            resolve_cb.as_ref().unchecked_ref(),
            Some(reject_cb.as_ref().unchecked_ref()),
        );
        resolve_cb.forget();
        reject_cb.forget();
    });
    let position = JsFuture::from(promise).await.ok()?;
    let coords = js_sys::Reflect::get(&position, &JsValue::from_str("coords")).ok()?;
    let lat = js_sys::Reflect::get(&coords, &JsValue::from_str("latitude"))
        .ok()?
        .as_f64()?;
    let lon = js_sys::Reflect::get(&coords, &JsValue::from_str("longitude"))
        .ok()?
        .as_f64()?;
    Some((lat, lon))
}

/// Non-web builds have no geolocation provider; the button stays hidden, but
/// the function is kept so callers do not need to be cfg-gated.
#[cfg(not(target_arch = "wasm32"))]
#[allow(dead_code)]
pub async fn current_location() -> Option<(f64, f64)> {
    None
}

/// Toggle a `theme-light` class on the document root so the page (and the
/// area around the app) picks up the light palette. The app root also carries
/// the class, so this is only needed on the web to cover `<body>`.
#[cfg(target_arch = "wasm32")]
pub fn set_theme(light: bool) {
    if let Some(root) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.document_element())
    {
        let _ = root.class_list().toggle_with_force("theme-light", light);
    }
}

/// Non-web builds have no document root; the app root carries the theme class.
#[cfg(not(target_arch = "wasm32"))]
pub fn set_theme(_light: bool) {}

/// The language to start with, guessed from the browser, when the user has no
/// saved preference yet. Only offers a language this build actually ships.
#[cfg(target_arch = "wasm32")]
pub fn preferred_language() -> Option<benthic_core::Language> {
    let language = web_sys::window()?.navigator().language()?;
    if language.to_lowercase().starts_with("fr") {
        Some(benthic_core::Language::French)
    } else {
        None
    }
}

/// Native builds have no browser locale; the default applies.
#[cfg(not(target_arch = "wasm32"))]
pub fn preferred_language() -> Option<benthic_core::Language> {
    None
}

/// Keep the document's `lang` attribute in sync with the chosen language, so
/// screen readers and search engines announce the right locale.
#[cfg(target_arch = "wasm32")]
pub fn set_language(code: &str) {
    if let Some(root) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.document_element())
    {
        let _ = root.set_attribute("lang", code);
    }
}

/// Non-web builds have no document; the language is only a display preference.
#[cfg(not(target_arch = "wasm32"))]
pub fn set_language(_code: &str) {}

#[cfg(target_arch = "wasm32")]
pub fn save_file(filename: &str, contents: &str) -> Result<String, String> {
    use wasm_bindgen::JsCast;

    let array = js_sys::Array::new();
    array.push(&wasm_bindgen::JsValue::from_str(contents));
    let blob = web_sys::Blob::new_with_str_sequence(&array).map_err(|e| format!("{e:?}"))?;
    let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(|e| format!("{e:?}"))?;

    let window = web_sys::window().ok_or("no browser window")?;
    let document = window.document().ok_or("no browser document")?;
    let anchor = document.create_element("a").map_err(|e| format!("{e:?}"))?;
    let anchor = anchor
        .dyn_into::<web_sys::HtmlAnchorElement>()
        .map_err(|_| "created element was not an anchor".to_string())?;
    anchor.set_href(&url);
    anchor.set_download(filename);
    anchor.click();
    let _ = web_sys::Url::revoke_object_url(&url);
    Ok(format!("Downloaded {filename}"))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn save_file(filename: &str, contents: &str) -> Result<String, String> {
    let path = std::env::current_dir().unwrap_or_default().join(filename);
    std::fs::write(&path, contents).map_err(|e| e.to_string())?;
    Ok(format!("Saved to {}", path.display()))
}
