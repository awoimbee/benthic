//! URL paths for the app's panes and dialogs.
//!
//! The UI still keeps its state in signals; this module mirrors that state to
//! the browser history so every section has an address, Back/Forward work, and
//! links can be opened directly. Native builds keep the in-memory route but
//! have no URL bar to reflect it in.

use dioxus::prelude::WritableExt;

use crate::state::{AppState, Dialog};

/// A top-level destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// The main dive list.
    Dives,
    Trip(u32),
    Dialog(Dialog),
}

impl Route {
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub fn path(self) -> String {
        match self {
            Route::Dives => "/".to_string(),
            Route::Trip(id) => format!("/trips/{id}"),
            Route::Dialog(dialog) => format!("/{}", dialog.slug().unwrap_or_default()),
        }
    }

    /// The route implied by the UI: an addressable dialog takes precedence
    /// over the pane behind it.
    pub fn from_ui(dialog: Option<Dialog>, trip: Option<u32>) -> Self {
        match dialog {
            Some(dialog) if dialog.slug().is_some() => Route::Dialog(dialog),
            _ => trip.map_or(Route::Dives, Route::Trip),
        }
    }

    /// The dialog this route opens, if any.
    pub fn dialog(self) -> Option<Dialog> {
        match self {
            Route::Dialog(dialog) => Some(dialog),
            _ => None,
        }
    }
}

/// Apply a route to the UI signals (used for Back/Forward and direct links).
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn apply_route(state: AppState, route: Route) {
    let mut dialog = state.dialog;
    let mut show_trip = state.show_trip;
    let mut mobile_detail = state.mobile_detail;
    dialog.set(route.dialog());
    match route {
        Route::Dives => {
            show_trip.set(None);
            mobile_detail.set(false);
        }
        Route::Trip(id) => {
            show_trip.set(Some(id));
            mobile_detail.set(true);
        }
        Route::Dialog(_) => {}
    }
}

/// Parse a pathname into a route, ignoring any deployment base path.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn route_from_path(path: &str) -> Route {
    let path = path.split(['?', '#']).next().unwrap_or(path);
    let parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    let Some(&last) = parts.last() else {
        return Route::Dives;
    };
    if let Some(dialog) = Dialog::from_slug(last) {
        return Route::Dialog(dialog);
    }
    if parts.len() >= 2 && parts[parts.len() - 2] == "trips" {
        if let Ok(id) = last.parse() {
            return Route::Trip(id);
        }
    }
    Route::Dives
}

#[cfg(target_arch = "wasm32")]
mod imp {
    use std::sync::OnceLock;

    use wasm_bindgen::JsValue;

    use super::Route;
    use crate::state::Dialog;

    fn location_path() -> String {
        web_sys::window()
            .and_then(|window| window.location().pathname().ok())
            .unwrap_or_else(|| "/".to_string())
    }

    /// The deployment prefix (`/benthic` on GitHub Pages, empty at a domain
    /// root), inferred once from the first URL we see.
    fn base_path() -> &'static str {
        static BASE: OnceLock<String> = OnceLock::new();
        BASE.get_or_init(|| {
            let path = location_path();
            let trimmed = path.trim_end_matches('/');
            let parts: Vec<&str> = trimmed.split('/').filter(|p| !p.is_empty()).collect();
            let take = match super::route_from_path(&path) {
                Route::Dives => 0,
                Route::Dialog(_) => 1,
                Route::Trip(_) => 2,
            };
            let base = parts[..parts.len().saturating_sub(take)].join("/");
            if base.is_empty() {
                String::new()
            } else {
                format!("/{base}")
            }
        })
    }

    fn full_path(route: Route) -> String {
        let base = base_path();
        let path = route.path();
        if base.is_empty() {
            path
        } else if path == "/" {
            format!("{base}/")
        } else {
            format!("{base}{path}")
        }
    }

    pub fn current_route() -> Route {
        // GitHub Pages serves deep links through `404.html`, which redirects
        // to the app root with the intended path in `?route=`.
        if let Some(window) = web_sys::window() {
            if let Ok(search) = window.location().search() {
                if let Some(value) = search.strip_prefix("?route=") {
                    if let Ok(decoded) = js_sys::decode_uri_component(value) {
                        if let Some(path) = decoded.as_string() {
                            let (route_path, query) = path
                                .split_once('?')
                                .map(|(path, query)| (path, Some(query)))
                                .unwrap_or((path.as_str(), None));
                            let route = super::route_from_path(route_path);
                            replace_query(route, query.unwrap_or(""));
                            return route;
                        }
                    }
                }
            }
        }
        super::route_from_path(&location_path())
    }

    pub fn current_query() -> Option<String> {
        let search = web_sys::window()?.location().search().ok()?;
        let query = search.strip_prefix('?').unwrap_or(&search);
        (!query.is_empty()).then(|| query.to_string())
    }

    pub fn replace_query(route: Route, query: &str) {
        if let Some(window) = web_sys::window() {
            let mut url = full_path(route);
            if !query.is_empty() {
                url.push('?');
                url.push_str(query);
            }
            let _ = window
                .history()
                .and_then(|history| history.replace_state_with_url(&JsValue::NULL, "", Some(&url)));
        }
    }

    pub fn push(route: Route) {
        if let Some(window) = web_sys::window() {
            let mut url = full_path(route);
            // The planner writes its parameters with replaceState. If it won
            // the race against this push, keep them on the new entry.
            if route == Route::Dialog(Dialog::Planner) {
                if let Ok(search) = window.location().search() {
                    url.push_str(&search);
                }
            }
            let _ = window
                .history()
                .and_then(|history| history.push_state_with_url(&JsValue::NULL, "", Some(&url)));
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    use super::Route;

    pub fn current_route() -> Route {
        Route::Dives
    }

    pub fn push(_route: Route) {}

    pub fn current_query() -> Option<String> {
        None
    }

    pub fn replace_query(_route: Route, _query: &str) {}
}

pub use imp::{current_query, current_route, push, replace_query};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_routes_with_and_without_a_base_path() {
        let planner = Route::Dialog(Dialog::Planner);
        assert_eq!(route_from_path("/planner"), planner);
        assert_eq!(route_from_path("/benthic/planner/"), planner);
        assert_eq!(
            route_from_path("/benthic/trips"),
            Route::Dialog(Dialog::Trips)
        );
        assert_eq!(
            route_from_path("/benthic/people"),
            Route::Dialog(Dialog::People)
        );
        assert_eq!(route_from_path("/benthic/trips/42"), Route::Trip(42));
        assert_eq!(route_from_path("/benthic/"), Route::Dives);
        assert_eq!(route_from_path("/"), Route::Dives);
        assert_eq!(route_from_path("/benthic/nope"), Route::Dives);
    }

    #[test]
    fn every_routable_dialog_round_trips() {
        for dialog in Dialog::ROUTABLE {
            let route = Route::Dialog(dialog);
            assert_eq!(route_from_path(&route.path()), route);
        }
        assert_eq!(route_from_path(&Route::Trip(7).path()), Route::Trip(7));
    }
}
