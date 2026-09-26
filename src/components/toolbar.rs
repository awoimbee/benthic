use dioxus::prelude::*;

use crate::components::ImportExport;
use crate::state::AppState;

#[component]
pub fn Toolbar() -> Element {
    let state = use_context::<AppState>();
    let status = (state.status)();
    let dive_count = (state.log)().dives.len();

    rsx! {
        header { class: "toolbar",
            span { class: "brand", "benthic" }
            span { class: "muted", "{dive_count} dives" }
            div { class: "spacer" }
            ImportExport {}
            span { class: "status", "{status}" }
        }
    }
}
