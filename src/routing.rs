//! URL paths for the app's panes and dialogs.
//!
//! The UI still keeps its state in signals; this module mirrors that state to
//! the browser history so every section has an address, Back/Forward work, and
//! links can be opened directly. Native builds keep the in-memory route but
//! have no URL bar to reflect it in.

use dioxus::prelude::WritableExt;

use crate::state::AppState;

/// A top-level destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// The main dive list.
    Dives,
    Planner,
    Map,
    Trips,
    Trip(u32),
    Preferences,
    Sync,
    Compare,
    Download,
}

impl Route {
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub fn path(self) -> String {
        match self {
            Route::Dives => "/".to_string(),
            Route::Planner => "/planner".to_string(),
            Route::Map => "/map".to_string(),
            Route::Trips => "/trips".to_string(),
            Route::Trip(id) => format!("/trips/{id}"),
            Route::Preferences => "/preferences".to_string(),
            Route::Sync => "/sync".to_string(),
            Route::Compare => "/compare".to_string(),
            Route::Download => "/download".to_string(),
        }
    }

    /// The route implied by the current UI signals. Dialogs take precedence
    /// over the pane behind them.
    #[allow(clippy::too_many_arguments)]
    pub fn from_ui(
        prefs: bool,
        trips: bool,
        map: bool,
        planner: bool,
        sync: bool,
        compare: bool,
        download: bool,
        trip: Option<u32>,
    ) -> Self {
        if planner {
            Route::Planner
        } else if prefs {
            Route::Preferences
        } else if trips {
            Route::Trips
        } else if map {
            Route::Map
        } else if sync {
            Route::Sync
        } else if compare {
            Route::Compare
        } else if download {
            Route::Download
        } else if let Some(id) = trip {
            Route::Trip(id)
        } else {
            Route::Dives
        }
    }
}

/// Apply a route to the UI signals (used for Back/Forward and direct links).
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn apply_route(state: AppState, route: Route) {
    let mut show_prefs = state.show_prefs;
    let mut show_trips = state.show_trips;
    let mut show_map = state.show_map;
    let mut show_planner = state.show_planner;
    let mut show_sync = state.show_sync;
    let mut show_compare = state.show_compare;
    let mut show_download = state.show_download;
    let mut show_trip = state.show_trip;
    let mut mobile_detail = state.mobile_detail;

    show_prefs.set(route == Route::Preferences);
    show_trips.set(route == Route::Trips);
    show_map.set(route == Route::Map);
    show_planner.set(route == Route::Planner);
    show_sync.set(route == Route::Sync);
    show_compare.set(route == Route::Compare);
    show_download.set(route == Route::Download);

    match route {
        Route::Dives => {
            show_trip.set(None);
            mobile_detail.set(false);
        }
        Route::Trip(id) => {
            show_trip.set(Some(id));
            mobile_detail.set(true);
        }
        _ => {}
    }
}

/// Parse a pathname into a route, ignoring any deployment base path.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn route_from_path(path: &str) -> Route {
    let path = path
        .split(['?', '#'])
        .next()
        .unwrap_or(path)
        .trim_end_matches('/');
    let parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    let Some(last) = parts.last().copied() else {
        return Route::Dives;
    };
    match last {
        "planner" => Route::Planner,
        "map" => Route::Map,
        "preferences" => Route::Preferences,
        "sync" => Route::Sync,
        "compare" => Route::Compare,
        "download" => Route::Download,
        "trips" => Route::Trips,
        id if parts.len() >= 2 && parts[parts.len() - 2] == "trips" => {
            id.parse().map(Route::Trip).unwrap_or(Route::Dives)
        }
        _ => Route::Dives,
    }
}

#[cfg(target_arch = "wasm32")]
mod imp {
    use std::sync::OnceLock;

    use wasm_bindgen::JsValue;

    use super::Route;

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
            let take = match parts.as_slice() {
                [] => 0,
                [.., "planner"] => 1,
                [.., "map"] => 1,
                [.., "preferences"] => 1,
                [.., "sync"] => 1,
                [.., "compare"] => 1,
                [.., "download"] => 1,
                [.., "trips"] => 1,
                [.., "trips", _] => 2,
                _ => 0,
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
            if route == Route::Planner {
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
        assert_eq!(route_from_path("/planner"), Route::Planner);
        assert_eq!(route_from_path("/benthic/planner/"), Route::Planner);
        assert_eq!(route_from_path("/benthic/trips"), Route::Trips);
        assert_eq!(route_from_path("/benthic/trips/42"), Route::Trip(42));
        assert_eq!(route_from_path("/benthic/"), Route::Dives);
        assert_eq!(route_from_path("/"), Route::Dives);
        assert_eq!(route_from_path("/benthic/nope"), Route::Dives);
    }
}
