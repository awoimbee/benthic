//! Renumber every dive chronologically from a chosen starting number.

use dioxus::prelude::*;

use crate::actions;
use crate::i18n;
use crate::state::AppState;

#[component]
pub fn RenumberDialog() -> Element {
    let state = use_context::<AppState>();
    let tr = state.strings();
    let log = (state.log)();
    // Default to the current lowest number, so renumbering only closes gaps
    // and fixes the order unless the diver asks for a different start.
    let mut start = use_signal(|| {
        log.dives
            .iter()
            .map(|dive| dive.number)
            .filter(|number| *number > 0)
            .min()
            .unwrap_or(1)
    });

    let first = start().max(1);
    let count = log.dives.len();
    let last = first.saturating_add(count.saturating_sub(1) as i32);
    let changed = {
        let mut preview = log.clone();
        preview.renumber(first)
    };

    rsx! {
        div { class: "modal-backdrop", onclick: move |_| state.close_dialog(),
            div {
                class: "modal",
                role: "dialog",
                tabindex: "-1",
                autofocus: true,
                aria_modal: "true",
                aria_label: "{tr.renumber_title}",
                onclick: move |evt| evt.stop_propagation(),
                h2 { "{tr.renumber_title}" }
                p { class: "muted", "{tr.renumber_hint}" }
                div { class: "edit-form",
                    label { class: "field-label", "{tr.renumber_start}"
                        input {
                            class: "field",
                            r#type: "number",
                            min: "1",
                            value: "{first}",
                            oninput: move |evt| {
                                if let Ok(value) = evt.value().parse::<i32>() {
                                    start.set(value.max(1));
                                }
                            },
                        }
                    }
                }
                p { class: "muted",
                    {i18n::fill(tr.renumber_preview, &[count.to_string(), first.to_string(), last.to_string(), changed.to_string()])}
                }
                div { class: "detail-actions",
                    button {
                        class: "btn primary",
                        disabled: changed == 0,
                        onclick: move |_| actions::renumber_dives(state, first),
                        "{tr.renumber_apply}"
                    }
                    button { class: "btn", onclick: move |_| state.close_dialog(), "{tr.cancel}" }
                }
            }
        }
    }
}
