use dioxus::prelude::*;

use crate::state::AppState;

/// A tappable "?" that reveals a short explanation.
///
/// `title` tooltips never appear on touch devices, so anywhere the app explains
/// jargon it should use this instead (or as well).
///
/// A click or tap anywhere outside the explanation closes it. The explanation
/// is first laid out off-flow to measure it, then shifted so it stays inside
/// the window instead of widening the page.
#[component]
pub fn InfoTip(text: &'static str) -> Element {
    let state = use_context::<AppState>();
    let t = crate::i18n::strings((state.prefs)().language);
    let mut open = use_signal(|| false);
    let mut anchor_left = use_signal(|| None::<f64>);
    let mut viewport_width = use_signal(|| None::<f64>);
    let mut pop_width = use_signal(|| None::<f64>);

    if !(open)() {
        return rsx! {
            button {
                class: "info-tip",
                r#type: "button",
                aria_label: "{t.more_info}",
                onclick: move |evt| {
                    evt.stop_propagation();
                    anchor_left.set(None);
                    viewport_width.set(None);
                    pop_width.set(None);
                    open.set(true);
                },
                "?"
            }
        };
    }

    const MARGIN: f64 = 8.0;
    let offset = match (anchor_left(), viewport_width(), pop_width()) {
        (Some(left), Some(viewport), Some(width)) => {
            let max = viewport - MARGIN - width - left;
            Some(max.min(0.0).max(MARGIN - left))
        }
        _ => None,
    };
    let (pop_class, pop_style) = match offset {
        Some(offset) => ("info-pop", format!("left: {offset:.1}px;")),
        None => ("info-pop measuring", String::new()),
    };

    rsx! {
        span {
            class: "info-tip-wrap",
            onmounted: move |evt: MountedEvent| async move {
                if let Ok(rect) = evt.get_client_rect().await {
                    anchor_left.set(Some(rect.origin.x));
                }
            },
            div {
                class: "info-tip-backdrop",
                onmounted: move |evt: MountedEvent| async move {
                    if let Ok(rect) = evt.get_client_rect().await {
                        viewport_width.set(Some(rect.size.width));
                    }
                },
                onclick: move |evt| {
                    evt.stop_propagation();
                    open.set(false);
                },
            }
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
                class: "{pop_class}",
                style: "{pop_style}",
                onmounted: move |evt: MountedEvent| async move {
                    if let Ok(rect) = evt.get_client_rect().await {
                        pop_width.set(Some(rect.size.width));
                    }
                },
                onclick: move |evt| evt.stop_propagation(),
                "{text}"
            }
        }
    }
}
