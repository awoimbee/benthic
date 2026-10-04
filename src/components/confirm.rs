use dioxus::prelude::*;

/// A small yes/no dialog for destructive actions. The caller decides what the
/// buttons mean via the two event handlers.
#[component]
pub fn ConfirmDialog(
    title: String,
    body: String,
    confirm_label: String,
    cancel_label: String,
    on_confirm: EventHandler<()>,
    on_cancel: EventHandler<()>,
) -> Element {
    rsx! {
        div { class: "modal-backdrop", onclick: move |_| on_cancel.call(()),
            div {
                class: "modal confirm",
                role: "dialog",
                aria_modal: "true",
                aria_label: "{title}",
                onclick: move |evt| evt.stop_propagation(),
                h2 { "{title}" }
                p { class: "muted", "{body}" }
                div { class: "detail-actions",
                    button {
                        class: "btn danger",
                        onclick: move |_| on_confirm.call(()),
                        "{confirm_label}"
                    }
                    button {
                        class: "btn",
                        onclick: move |_| on_cancel.call(()),
                        "{cancel_label}"
                    }
                }
            }
        }
    }
}
