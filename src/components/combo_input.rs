//! A text input with a themed suggestion list, used instead of a native
//! `<datalist>`, whose popup cannot be styled or positioned.

use std::sync::atomic::{AtomicUsize, Ordering};

use dioxus::prelude::*;

static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

/// A text input that suggests `options` as the user types.
///
/// With `multi`, the value is a comma-separated list: suggestions complete the
/// last entry and skip entries already present.
#[component]
pub fn ComboInput(
    value: String,
    options: Vec<String>,
    #[props(default)] multi: bool,
    oninput: EventHandler<String>,
) -> Element {
    let list_id = use_hook(|| format!("combo-{}", NEXT_ID.fetch_add(1, Ordering::Relaxed)));
    let mut open = use_signal(|| false);
    let mut highlight = use_signal(|| None::<usize>);

    let (head, query) = split_last(&value, multi);
    let query_lower = query.to_lowercase();
    let matches: Vec<String> = options
        .iter()
        .filter(|option| {
            let lower = option.to_lowercase();
            lower.contains(&query_lower)
                && lower != query_lower
                && !head.iter().any(|entry| entry.eq_ignore_ascii_case(option))
        })
        .cloned()
        .collect();
    let shown = open() && !matches.is_empty();
    let active = highlight().filter(|index| *index < matches.len());

    let choose = {
        let head = head.clone();
        move |choice: String| {
            let mut entries = head.clone();
            entries.push(choice);
            oninput.call(entries.join(", "));
            open.set(false);
            highlight.set(None);
        }
    };

    let on_keydown = {
        let matches = matches.clone();
        let mut choose = choose.clone();
        move |evt: KeyboardEvent| {
            let count = matches.len();
            match evt.key() {
                Key::ArrowDown if count > 0 => {
                    evt.prevent_default();
                    open.set(true);
                    highlight.set(Some(active.map_or(0, |index| (index + 1) % count)));
                }
                Key::ArrowUp if count > 0 => {
                    evt.prevent_default();
                    open.set(true);
                    highlight.set(Some(
                        active.map_or(count - 1, |index| (index + count - 1) % count),
                    ));
                }
                Key::Enter if shown => {
                    if let Some(index) = active {
                        evt.prevent_default();
                        choose(matches[index].clone());
                    }
                }
                Key::Escape if shown => {
                    evt.prevent_default();
                    evt.stop_propagation();
                    open.set(false);
                }
                _ => {}
            }
        }
    };

    rsx! {
        div { class: "combo",
            input {
                class: "field",
                role: "combobox",
                autocomplete: "off",
                aria_autocomplete: "list",
                aria_expanded: if shown { "true" } else { "false" },
                aria_controls: "{list_id}",
                aria_activedescendant: active.map(|index| format!("{list_id}-{index}")),
                value: "{value}",
                oninput: move |evt| {
                    open.set(true);
                    highlight.set(None);
                    oninput.call(evt.value());
                },
                onfocus: move |_| open.set(true),
                onblur: move |_| open.set(false),
                onkeydown: on_keydown,
            }
            if shown {
                ul { class: "combo-list", id: "{list_id}", role: "listbox",
                    for (index, option) in matches.into_iter().enumerate() {
                        li {
                            key: "{option}",
                            id: "{list_id}-{index}",
                            class: if active == Some(index) { "combo-option active" } else { "combo-option" },
                            role: "option",
                            aria_selected: if active == Some(index) { "true" } else { "false" },
                            // Keep focus in the input so the click lands.
                            onmousedown: move |evt| evt.prevent_default(),
                            onmouseenter: move |_| highlight.set(Some(index)),
                            onclick: {
                                let mut choose = choose.clone();
                                let option = option.clone();
                                move |_| choose(option.clone())
                            },
                            "{option}"
                        }
                    }
                }
            }
        }
    }
}

/// Split `value` into the completed entries and the one being typed.
fn split_last(value: &str, multi: bool) -> (Vec<String>, String) {
    if !multi {
        return (Vec::new(), value.trim().to_string());
    }
    let mut parts: Vec<String> = value
        .split(',')
        .map(|part| part.trim().to_string())
        .collect();
    let last = parts.pop().unwrap_or_default();
    parts.retain(|part| !part.is_empty());
    (parts, last)
}

#[cfg(test)]
mod tests {
    use super::split_last;

    #[test]
    fn splits_the_entry_being_typed() {
        assert_eq!(split_last(" Paul ", false), (vec![], "Paul".to_string()));
        assert_eq!(
            split_last("Camille, Pa", true),
            (vec!["Camille".to_string()], "Pa".to_string())
        );
        assert_eq!(
            split_last("Camille, ", true),
            (vec!["Camille".to_string()], String::new())
        );
    }
}
