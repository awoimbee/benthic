//! Platform-specific file actions: saving an export.
//!
//! Reading imports is handled uniformly by Dioxus' file input + `FileData`,
//! so only "save to disk / trigger a download" needs per-platform code.

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
