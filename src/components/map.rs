//! Dive-site map view.
//!
//! Rendering is delegated to [Leaflet](https://leafletjs.com/) with the
//! marker-cluster plugin. Both are vendored into `assets/map/` (Leaflet is
//! BSD-2-Clause, the cluster plugin MIT) so the app never has to fetch its
//! controls from a third-party CDN; only the map tiles themselves are remote.
//! Satellite imagery comes from Esri's World Imagery service and the street
//! layer from OpenStreetMap.
//!
//! The Rust side owns no map state: it serialises the sites and lets the
//! injected script do the drawing, receiving marker clicks back through the
//! [`document::eval`] channel.

use dioxus::prelude::*;
use serde::Serialize;

use crate::i18n;
use crate::state::AppState;

const MAP_CSS: Asset = asset!("/assets/map/leaflet.bundle.css");
// Vendored UMD libraries: ship them verbatim rather than letting the JS
// bundler tree-shake the global assignments.
const MAP_JS: Asset = asset!(
    "/assets/map/leaflet.bundle.js",
    AssetOptions::js().with_minify(false)
);

/// A dive site projected onto the map.
#[derive(Clone, PartialEq, Serialize)]
pub struct MapSite {
    /// Site uuid, echoed back when its marker is clicked.
    pub site_id: u32,
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    /// The most recent dive at this site, if any, so a click can open it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dive_id: Option<u32>,
    pub dives: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
}

impl MapSite {
    /// Collect every geolocated site in the log, most recently dived first.
    pub fn all(log: &benthic_core::DiveLog) -> Vec<MapSite> {
        let mut sites: Vec<MapSite> = log
            .sites
            .iter()
            .filter_map(|site| {
                let location = site.location.filter(|l| l.is_valid())?;
                let dives: Vec<&benthic_core::Dive> = log
                    .dives
                    .iter()
                    .filter(|d| d.site_id == Some(site.uuid))
                    .collect();
                let dive_id = dives.iter().max_by_key(|d| d.when).map(|d| d.id);
                Some(MapSite {
                    site_id: site.uuid,
                    name: if site.name.is_empty() {
                        "Unnamed site".to_string()
                    } else {
                        site.name.clone()
                    },
                    lat: location.lat,
                    lon: location.lon,
                    dive_id,
                    dives: dives.len(),
                    country: site.country.clone(),
                })
            })
            .collect();
        sites.sort_by(|a, b| b.dives.cmp(&a.dives).then_with(|| a.name.cmp(&b.name)));
        sites
    }
}

fn next_instance() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// An interactive Leaflet map of dive sites.
#[component]
pub fn MapView(
    sites: Vec<MapSite>,
    #[props(default)] max_zoom: u8,
    #[props(default)] height: Option<String>,
    #[props(default)] on_select: Option<EventHandler<u32>>,
    /// When true, tapping the map reports the tapped coordinates instead of
    /// (or as well as) opening a marker.
    #[props(default)]
    pick: bool,
    #[props(default)] on_pick: Option<EventHandler<(f64, f64)>>,
) -> Element {
    let state = use_context::<AppState>();
    let t = i18n::strings((state.prefs)().language);
    let dom_id = use_hook(|| format!("benthic-map-{}", next_instance()));
    let mount_id = dom_id.clone();
    // The Leaflet instance lives outside Dioxus. A component that is the single
    // root of its parent is diffed in place (keys are only honoured for
    // fragments), so we cannot rely on remounting when the selected site
    // changes. Instead we keep the eval handle and push updated marker data
    // into the existing map.
    let mut eval_slot = use_signal(|| None::<document::Eval>);
    use_effect(use_reactive((&sites,), move |(sites,)| match eval_slot() {
        Some(eval) => {
            let _ = eval.send(serde_json::json!({ "type": "sites", "sites": sites }));
        }
        None => {
            let payload = serde_json::json!({
                "id": mount_id.clone(),
                "sites": sites,
                "max_zoom": max_zoom,
                "pick": pick,
                "satellite": t.satellite,
                "streets": t.streets,
            });
            let mut eval = document::eval(MAP_SCRIPT);
            if eval.send(payload).is_err() {
                return;
            }
            eval_slot.set(Some(eval));
            spawn(async move {
                while let Ok(message) = eval.recv::<String>().await {
                    let Ok(value) = serde_json::from_str::<serde_json::Value>(&message) else {
                        continue;
                    };
                    match value.get("type").and_then(|t| t.as_str()) {
                        Some("select") => {
                            if let (Some(handler), Some(site)) = (
                                on_select,
                                value.get("site").and_then(|s| s.as_u64()).map(|s| s as u32),
                            ) {
                                handler.call(site);
                            }
                        }
                        Some("pick") => {
                            if let Some(handler) = on_pick {
                                let lat = value.get("lat").and_then(|v| v.as_f64());
                                let lon = value.get("lon").and_then(|v| v.as_f64());
                                if let (Some(lat), Some(lon)) = (lat, lon) {
                                    handler.call((lat, lon));
                                }
                            }
                        }
                        _ => {}
                    }
                }
            });
        }
    }));

    let style = height.map(|h| format!("height: {h};"));
    rsx! {
        document::Stylesheet { href: MAP_CSS }
        document::Script { src: MAP_JS }
        div { id: "{dom_id}", class: "benthic-map", style: style.as_deref().unwrap_or("") }
    }
}

/// A modal map used to pick one coordinate for a dive site.
#[component]
pub fn LocationPickerDialog(
    on_choose: EventHandler<(f64, f64)>,
    on_close: EventHandler<()>,
) -> Element {
    let state = use_context::<AppState>();
    let tr = i18n::strings((state.prefs)().language);
    let mut picked = use_signal(|| None::<(f64, f64)>);
    let sites: Vec<MapSite> = picked()
        .map(|(lat, lon)| {
            vec![MapSite {
                site_id: 0,
                name: tr.selected_location.to_string(),
                lat,
                lon,
                dive_id: None,
                dives: 0,
                country: None,
            }]
        })
        .unwrap_or_default();

    rsx! {
        div { class: "modal-backdrop", onclick: move |_| on_close.call(()),
            div { class: "modal map-modal",
                role: "dialog",
                aria_modal: "true",
                aria_label: "{tr.pick_title}",
                onclick: move |evt| evt.stop_propagation(),
                div { class: "map-modal-head",
                    h2 { "{tr.pick_title}" }
                    span { class: "muted", "{tr.pick_hint}" }
                }
                MapView {
                    sites: sites.clone(),
                    max_zoom: 16,
                    height: "340px".to_string(),
                    pick: true,
                    on_pick: move |(lat, lon)| picked.set(Some((lat, lon))),
                }
                div { class: "detail-actions",
                    button {
                        class: "btn primary",
                        disabled: picked().is_none(),
                        onclick: move |_| {
                            if let Some((lat, lon)) = picked() {
                                on_choose.call((lat, lon));
                            }
                        },
                        "{tr.use_this_location}"
                    }
                    button { class: "btn", onclick: move |_| on_close.call(()), "{tr.cancel}" }
                }
            }
        }
    }
}

struct SiteRow {
    id: u32,
    name: String,
    dives_label: String,
    dive_id: Option<u32>,
    country: Option<String>,
}

/// A modal map of every dive site, with a list to jump straight to a dive.
#[component]
pub fn MapDialog() -> Element {
    let state = use_context::<AppState>();
    let mut show_map = state.show_map;
    let mut selected = state.selected;
    let mut mobile_detail = state.mobile_detail;

    let log = (state.log)();
    let tr = i18n::strings((state.prefs)().language);
    let sites = MapSite::all(&log);
    let geolocated = sites.len();
    let total = log.sites.len();

    let mut open_dive = move |dive_id: Option<u32>| {
        let t = state.strings();
        if let Some(id) = dive_id {
            selected.set(Some(id));
            mobile_detail.set(true);
            show_map.set(false);
            state.set_status(t.opened_from_map);
        } else {
            state.set_status(t.no_dives_at_site);
        }
    };

    let rows: Vec<SiteRow> = sites
        .iter()
        .map(|s| SiteRow {
            id: s.site_id,
            name: s.name.clone(),
            dives_label: format!(
                "{} {}",
                s.dives,
                if s.dives == 1 {
                    tr.dive_singular
                } else {
                    tr.dive_plural
                }
            ),
            dive_id: s.dive_id,
            country: s.country.clone(),
        })
        .collect();

    rsx! {
        div {
            class: "modal-backdrop",
            onclick: move |_| show_map.set(false),
            div {
                class: "modal map-modal",
                role: "dialog",
                aria_modal: "true",
                aria_label: "{tr.map_title}",
                onclick: move |evt| evt.stop_propagation(),
                div { class: "map-modal-head",
                    h2 { "{tr.map_title}" }
                    span { class: "muted", {i18n::t2(tr.map_sites_on_map, geolocated, total)} }
                }
                div { class: "map-modal-body",
                    div { class: "map-site-list",
                        if rows.is_empty() {
                            div { class: "muted", "{tr.map_empty}" }
                        }
                        for row in rows {
                            button {
                                key: "{row.id}",
                                class: "map-site-row",
                                disabled: row.dive_id.is_none(),
                                onclick: move |_| open_dive(row.dive_id),
                                span { class: "map-site-name", "{row.name}" }
                                span { class: "muted map-site-meta",
                                    if let Some(country) = row.country.clone() {
                                        "{country} · "
                                    }
                                    "{row.dives_label}"
                                }
                            }
                        }
                    }
                    MapView {
                        sites: sites.clone(),
                        max_zoom: 12,
                        on_select: move |site: u32| {
                            let dive_id = sites
                                .iter()
                                .find(|s| s.site_id == site)
                                .and_then(|s| s.dive_id);
                            open_dive(dive_id);
                        },
                    }
                }
                div { class: "detail-actions",
                    button {
                        class: "btn",
                        onclick: move |_| show_map.set(false),
                        "{tr.close}"
                    }
                }
            }
        }
    }
}

/// The JavaScript driver. It waits for the vendored Leaflet bundle, mounts a map
/// on the container div, plots the sites with clustering, and forwards marker
/// clicks back to Rust.
const MAP_SCRIPT: &str = r##"
function waitForLeaflet() {
  return new Promise((resolve) => {
    let attempts = 0;
    const probe = () => {
      if (window.L && typeof window.L.map === "function") {
        resolve(true);
      } else if (attempts++ > 600) {
        resolve(false);
      } else {
        setTimeout(probe, 25);
      }
    };
    probe();
  });
}

function waitForElement(id) {
  return new Promise((resolve) => {
    let attempts = 0;
    const probe = () => {
      const element = document.getElementById(id);
      if (element) {
        resolve(element);
      } else if (attempts++ > 200) {
        resolve(null);
      } else {
        setTimeout(probe, 25);
      }
    };
    setTimeout(probe, 0);
  });
}

function escapeHtml(value) {
  return String(value ?? "").replace(/[&<>"']/g, (c) => ({
    "&": "&amp;",
    "<": "&lt;",
    ">": "&gt;",
    '"': "&quot;",
    "'": "&#39;",
  }[c]));
}

const payload = await dioxus.recv();
const ready = await waitForLeaflet();
const container = payload ? await waitForElement(payload.id) : null;

if (!ready || !container) {
  dioxus.send(JSON.stringify({
    type: "error",
    message: ready ? "map container is missing" : "map library failed to load",
  }));
} else {
  const map = L.map(container, { worldCopyJump: true, minZoom: 2 });

  const satellite = L.tileLayer(
    "https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}",
    {
      maxZoom: 19,
      attribution:
        "Tiles &copy; Esri &mdash; Source: Esri, Maxar, Earthstar Geographics, and the GIS User Community",
    },
  );
  const streets = L.tileLayer("https://{s}.tile.openstreetmap.org/{z}/{x}/{y}.png", {
    maxZoom: 19,
    attribution:
      '&copy; <a href="https://www.openstreetmap.org/copyright">OpenStreetMap</a> contributors',
  });
  satellite.addTo(map);

  const LayerToggle = L.Control.extend({
    options: { position: "topright" },
    onAdd() {
      const div = L.DomUtil.create("div", "benthic-map-layers");
      div.innerHTML =
        '<button type="button" class="active" data-layer="satellite">' +
        escapeHtml(payload.satellite || "Satellite") +
        '</button><button type="button" data-layer="streets">' +
        escapeHtml(payload.streets || "Streets") +
        "</button>";
      L.DomEvent.disableClickPropagation(div);
      L.DomEvent.disableScrollPropagation(div);
      L.DomEvent.on(div, "click", (event) => {
        const button = event.target.closest("button");
        if (!button) return;
        if (button.getAttribute("data-layer") === "satellite") {
          map.removeLayer(streets);
          satellite.addTo(map);
        } else {
          map.removeLayer(satellite);
          streets.addTo(map);
        }
        div.querySelectorAll("button").forEach((candidate) => {
          candidate.classList.toggle("active", candidate === button);
        });
      });
      return div;
    },
  });
  map.addControl(new LayerToggle());

  if (payload.pick) {
    map.on("click", (event) => {
      dioxus.send(
        JSON.stringify({ type: "pick", lat: event.latlng.lat, lon: event.latlng.lng }),
      );
    });
  }

  const group =
    typeof L.markerClusterGroup === "function"
      ? L.markerClusterGroup({ showCoverageOnHover: false, maxClusterRadius: 45 })
      : L.layerGroup();
  map.addLayer(group);

  const icon = L.divIcon({
    className: "benthic-site-icon",
    html: "<span></span>",
    iconSize: [18, 18],
    iconAnchor: [9, 9],
  });

  const addSites = (sites) => {
    group.clearLayers();
    const points = [];
    for (const site of sites || []) {
      const marker = L.marker([site.lat, site.lon], { title: site.name, icon });
      const country = site.country
        ? '<div class="benthic-map-popup-meta">' + escapeHtml(site.country) + "</div>"
        : "";
      const dives = site.dives
        ? '<div class="benthic-map-popup-meta">' +
          site.dives +
          " dive" +
          (site.dives === 1 ? "" : "s") +
          "</div>"
        : "";
      marker.bindPopup(
        "<strong>" + escapeHtml(site.name) + "</strong>" + country + dives,
      );
      marker.on("click", () => {
        dioxus.send(JSON.stringify({ type: "select", site: site.site_id }));
      });
      group.addLayer(marker);
      points.push([site.lat, site.lon]);
    }
    const maxZoom = payload.max_zoom || 12;
    if (points.length === 1) {
      map.setView(points[0], Math.min(maxZoom, 13));
    } else if (points.length > 1) {
      map.fitBounds(points, { maxZoom, padding: [30, 30] });
    } else {
      map.setView([20, 0], 2);
    }
  };

  addSites(payload.sites);
  setTimeout(() => map.invalidateSize(), 0);

  while (true) {
    const message = await dioxus.recv();
    if (!message || message.type === "done") break;
    if (message.type === "sites") addSites(message.sites);
    else if (message.type === "focus") map.setView([message.lat, message.lon], message.zoom || 12);
    else if (message.type === "resize") map.invalidateSize();
  }
  map.remove();
}
"##;

#[cfg(test)]
mod tests {
    use super::*;
    use benthic_core::{Dive, DiveLog, DiveSite, Location};

    #[test]
    fn map_sites_skip_missing_coordinates_and_pick_latest_dive() {
        let mut log = DiveLog::new();
        let mut reef = DiveSite {
            uuid: 1,
            name: "Reef".to_string(),
            ..Default::default()
        };
        reef.location = Some(Location::new(28.5, 34.5));
        log.sites.push(reef);
        log.sites.push(DiveSite {
            uuid: 2,
            name: "Unknown".to_string(),
            ..Default::default()
        });

        let mut older = Dive::manual(100);
        older.id = 10;
        older.site_id = Some(1);
        let mut newer = Dive::manual(200);
        newer.id = 11;
        newer.site_id = Some(1);
        log.dives.push(older);
        log.dives.push(newer);

        let sites = MapSite::all(&log);
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].site_id, 1);
        assert_eq!(sites[0].dives, 2);
        assert_eq!(sites[0].dive_id, Some(11));
    }
}
