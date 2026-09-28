//! Web OAuth: the Google Identity Services bridge.
//!
//! `public/google-auth.js` loads GIS on demand and exposes
//! `benthicGoogleLogin`, which runs the browser token flow and resolves with a
//! JSON string. This module turns that into a typed result.

use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(catch, js_name = benthicGoogleLogin)]
    fn js_google_login(client_id: &str, scope: &str) -> Result<js_sys::Promise, JsValue>;

    #[wasm_bindgen(js_name = benthicGoogleSignOut)]
    fn js_google_sign_out(token: &str);
}

/// Run the sign-in flow, returning `(access_token, expires_at_unix_secs)`.
pub async fn sign_in(client_id: &str, scope: &str) -> Result<(String, i64), String> {
    let promise = js_google_login(client_id, scope).map_err(js_message)?;
    let value = JsFuture::from(promise).await.map_err(js_message)?;
    let json = value
        .as_string()
        .ok_or_else(|| "Google sign-in returned an unexpected value.".to_string())?;
    let parsed: serde_json::Value =
        serde_json::from_str(&json).map_err(|error| error.to_string())?;
    let token = parsed["access_token"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    if token.is_empty() {
        return Err("Google sign-in did not return a token.".to_string());
    }
    let expires_in = parsed["expires_in"].as_i64().unwrap_or(3600);
    Ok((token, crate::platform::now_secs() + expires_in))
}

/// Best-effort token revocation.
pub fn sign_out(token: &str) {
    js_google_sign_out(token);
}

/// Turn a rejected JS value (usually an `Error`) into a message.
fn js_message(value: JsValue) -> String {
    if let Some(message) = value.as_string() {
        return message;
    }
    js_sys::Reflect::get(&value, &JsValue::from_str("message"))
        .ok()
        .and_then(|message| message.as_string())
        .unwrap_or_else(|| "Google sign-in failed.".to_string())
}
