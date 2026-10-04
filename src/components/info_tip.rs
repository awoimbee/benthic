use dioxus::prelude::*;

use crate::state::AppState;

/// A tappable "?" that reveals a short explanation.
///
/// `title` tooltips never appear on touch devices, so anywhere the app explains
/// jargon it should use this instead (or as well).
#[component]
pub fn InfoTip(text: &'static str) -> Element {
    let state = use_context::<AppState>();
    let t = crate::i18n::strings((state.prefs)().language);
    let mut open = use_signal(|| false);

    if !(open)() {
        return rsx! {
            button {
                class: "info-tip",
                r#type: "button",
                aria_label: "{t.more_info}",
                onclick: move |evt| {
                    evt.stop_propagation();
                    open.set(true);
                },
                "?"
            }
        };
    }

    rsx! {
        span { class: "info-tip-wrap",
            button {
                class: "info-tip open",
                r#type: "button",
                aria_label: "{t.hide_info}",
                onclick: move |evt| {
                    evt.stop_propagation();
                    open.set(false);
                },
                "?"
            }
            span {
                class: "info-pop",
                onclick: move |evt| evt.stop_propagation(),
                "{text}"
            }
        }
    }
}
