//! The assistant chat panel, opened from the ✨ toolbar button.
//!
//! It sits beside the dive panes (full screen on phones) instead of being a
//! modal, so the diver can keep looking at a dive while asking about it.

use dioxus::core::Task;
use dioxus::prelude::*;

use crate::ai::{self, AiConfig, AiProvider, Entry, Thread};
use crate::state::AppState;

#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Chat,
    Threads,
    Settings,
}

#[component]
pub fn AssistantPanel() -> Element {
    let state = use_context::<AppState>();
    let tr = state.strings();
    let mut show_ai = state.show_ai;

    let mut config = use_signal(ai::load_config);
    let mut thread = use_signal(|| {
        ai::load_threads()
            .into_iter()
            .next()
            .unwrap_or_else(|| Thread::new(crate::platform::now_secs()))
    });
    let mut view = use_signal(|| {
        if ai::load_config().is_ready() {
            View::Chat
        } else {
            View::Settings
        }
    });
    let mut draft = use_signal(String::new);
    let mut busy = use_signal(|| false);
    // The answer as it streams in, before it is added to the thread.
    let mut partial = use_signal(String::new);
    let mut live_tools = use_signal(Vec::<(String, String)>::new);
    let mut task = use_signal(|| None::<Task>);

    let mut send = move |text: String| {
        let text = text.trim().to_string();
        let cfg = config();
        if text.is_empty() || busy() {
            return;
        }
        if !cfg.is_ready() {
            view.set(View::Settings);
            return;
        }
        draft.set(String::new());
        busy.set(true);
        partial.set(String::new());
        live_tools.set(Vec::new());
        thread
            .write()
            .entries
            .push(Entry::User { text: text.clone() });
        let context = ai::Context {
            log: (state.log)(),
            prefs: (state.prefs)(),
            selected: (state.selected)(),
        };
        let spawned = spawn(async move {
            let mut working = thread();
            let result = ai::ask(&cfg, context, &mut working, text, |event| match event {
                ai::Event::Text(delta) => partial.write().push_str(&delta),
                ai::Event::Tool { name, summary } => {
                    partial.set(String::new());
                    live_tools.write().push((name, summary));
                }
            })
            .await;
            for (name, summary) in live_tools() {
                working.entries.push(Entry::Tool { name, summary });
            }
            working.entries.push(match result {
                Ok(answer) => Entry::Assistant { text: answer },
                Err(error) => Entry::Error { text: error },
            });
            working.updated = crate::platform::now_secs();
            ai::save_thread(&working);
            thread.set(working);
            partial.set(String::new());
            live_tools.set(Vec::new());
            busy.set(false);
            task.set(None);
        });
        task.set(Some(spawned));
    };

    let stop = move |_| {
        if let Some(running) = task() {
            running.cancel();
        }
        let mut working = thread();
        let text = partial();
        if !text.trim().is_empty() {
            working.entries.push(Entry::Assistant { text });
        }
        working.entries.push(Entry::Error {
            text: tr.ai_stopped.to_string(),
        });
        thread.set(working);
        partial.set(String::new());
        live_tools.set(Vec::new());
        busy.set(false);
        task.set(None);
    };

    let new_thread = move |_| {
        if busy() {
            return;
        }
        thread.set(Thread::new(crate::platform::now_secs()));
        view.set(View::Chat);
    };

    let current_view = view();
    let current = thread();
    let threads = if current_view == View::Threads {
        ai::load_threads()
    } else {
        Vec::new()
    };
    let suggestions = [
        tr.ai_suggest_deepest,
        tr.ai_suggest_this_dive,
        tr.ai_suggest_year,
    ];

    rsx! {
        aside { class: "assistant", aria_label: "{tr.ai_assistant}",
            header { class: "assistant-head",
                span { class: "assistant-title", "✨ {tr.ai_assistant}" }
                div { class: "assistant-head-actions",
                    button {
                        class: "icon-btn",
                        r#type: "button",
                        title: "{tr.ai_new_thread}",
                        aria_label: "{tr.ai_new_thread}",
                        disabled: busy(),
                        onclick: new_thread,
                        "+"
                    }
                    button {
                        class: if current_view == View::Threads { "icon-btn active" } else { "icon-btn" },
                        r#type: "button",
                        title: "{tr.ai_threads}",
                        aria_label: "{tr.ai_threads}",
                        aria_pressed: if current_view == View::Threads { "true" } else { "false" },
                        onclick: move |_| view.set(if view() == View::Threads { View::Chat } else { View::Threads }),
                        "☰"
                    }
                    button {
                        class: if current_view == View::Settings { "icon-btn active" } else { "icon-btn" },
                        r#type: "button",
                        title: "{tr.ai_settings}",
                        aria_label: "{tr.ai_settings}",
                        aria_pressed: if current_view == View::Settings { "true" } else { "false" },
                        onclick: move |_| view.set(if view() == View::Settings { View::Chat } else { View::Settings }),
                        "⚙"
                    }
                    button {
                        class: "icon-btn",
                        r#type: "button",
                        title: "{tr.close}",
                        aria_label: "{tr.close}",
                        onclick: move |_| show_ai.set(false),
                        "✕"
                    }
                }
            }

            match current_view {
                View::Settings => rsx! {
                    AssistantSettings {
                        config: config(),
                        on_save: move |saved: AiConfig| {
                            let _ = ai::save_config(&saved);
                            config.set(saved);
                            view.set(View::Chat);
                        },
                    }
                },
                View::Threads => rsx! {
                    div { class: "assistant-body",
                        if threads.is_empty() {
                            p { class: "muted", "{tr.ai_no_threads}" }
                        }
                        for past in threads {
                            div { key: "{past.id}", class: "assistant-thread-row",
                                button {
                                    class: if past.id == current.id { "assistant-thread active" } else { "assistant-thread" },
                                    r#type: "button",
                                    onclick: {
                                        let past = past.clone();
                                        move |_| {
                                            if !busy() {
                                                thread.set(past.clone());
                                                view.set(View::Chat);
                                            }
                                        }
                                    },
                                    span { class: "assistant-thread-title", "{past.title().unwrap_or(\"…\")}" }
                                    span { class: "muted", "{(state.prefs)().timestamp(past.updated)}" }
                                }
                                button {
                                    class: "icon-btn",
                                    r#type: "button",
                                    title: "{tr.delete}",
                                    aria_label: "{tr.delete}",
                                    onclick: move |_| {
                                        ai::delete_thread(past.id);
                                        if thread().id == past.id {
                                            thread.set(Thread::new(crate::platform::now_secs()));
                                        }
                                        // Re-render the list.
                                        view.set(View::Threads);
                                    },
                                    "✕"
                                }
                            }
                        }
                    }
                },
                View::Chat => rsx! {
                    div { class: "assistant-body", role: "log", aria_live: "polite",
                        if current.is_empty() && !busy() {
                            div { class: "assistant-empty",
                                p { class: "muted", "{tr.ai_intro}" }
                                for suggestion in suggestions {
                                    button {
                                        class: "tag tag-add",
                                        r#type: "button",
                                        onclick: move |_| send(suggestion.to_string()),
                                        "{suggestion}"
                                    }
                                }
                            }
                        }
                        for (index, entry) in current.entries.iter().enumerate() {
                            EntryView { key: "{index}", entry: entry.clone() }
                        }
                        if busy() {
                            for (index, (name, summary)) in live_tools().into_iter().enumerate() {
                                EntryView { key: "live-tool-{index}", entry: Entry::Tool { name, summary } }
                            }
                            if partial().is_empty() {
                                div { class: "assistant-msg assistant-reply assistant-typing", "…" }
                            } else {
                                EntryView { key: "{current.entries.len()}-live", entry: Entry::Assistant { text: partial() } }
                            }
                        }
                        // Keeps the newest message in view.
                        div {
                            class: "assistant-end",
                            onmounted: move |evt: MountedEvent| async move {
                                let _ = evt.scroll_to(ScrollBehavior::Smooth).await;
                            },
                        }
                    }
                    form {
                        class: "assistant-compose",
                        onsubmit: move |evt: FormEvent| {
                            evt.prevent_default();
                            send(draft());
                        },
                        textarea {
                            class: "field",
                            rows: "2",
                            autofocus: true,
                            aria_label: "{tr.ai_prompt_placeholder}",
                            placeholder: "{tr.ai_prompt_placeholder}",
                            value: "{draft}",
                            oninput: move |evt| draft.set(evt.value()),
                            onkeydown: move |evt: KeyboardEvent| {
                                // Enter sends; Shift+Enter adds a line.
                                if evt.key() == Key::Enter && !evt.modifiers().contains(Modifiers::SHIFT) {
                                    evt.prevent_default();
                                    send(draft());
                                }
                            },
                        }
                        if busy() {
                            button { class: "btn", r#type: "button", onclick: stop, "{tr.ai_stop}" }
                        } else {
                            button {
                                class: "btn primary",
                                r#type: "submit",
                                disabled: draft().trim().is_empty(),
                                "{tr.ai_send}"
                            }
                        }
                    }
                },
            }
        }
    }
}

#[component]
fn EntryView(entry: Entry) -> Element {
    let state = use_context::<AppState>();
    let tr = state.strings();
    match entry {
        Entry::User { text } => rsx! {
            div { class: "assistant-msg assistant-question", "{text}" }
        },
        Entry::Assistant { text } => rsx! {
            div { class: "assistant-msg assistant-reply",
                for (index, paragraph) in text.split("\n\n").enumerate() {
                    p { key: "{index}",
                        for (part_index, part) in rich_text(paragraph).into_iter().enumerate() {
                            match part {
                                Part::Text(text) => rsx! { span { key: "{part_index}", "{text}" } },
                                Part::Bold(text) => rsx! { strong { key: "{part_index}", "{text}" } },
                                Part::Dive { label, id } => rsx! {
                                    button {
                                        key: "{part_index}",
                                        class: "assistant-dive-link",
                                        r#type: "button",
                                        onclick: move |_| open_dive(state, id),
                                        "{label}"
                                    }
                                },
                            }
                        }
                    }
                }
            }
        },
        Entry::Tool { name, summary } => {
            let label = match name.as_str() {
                "search_dives" => tr.ai_tool_search,
                "get_dive" => tr.ai_tool_get_dive,
                "log_stats" => tr.ai_tool_stats,
                "list_sites" => tr.ai_tool_sites,
                _ => name.as_str(),
            }
            .to_string();
            rsx! {
                div { class: "assistant-tool",
                    "🔎 {label}"
                    if !summary.is_empty() {
                        span { class: "muted", " · {summary}" }
                    }
                }
            }
        }
        Entry::Error { text } => rsx! {
            div { class: "assistant-msg assistant-error", role: "alert", "{text}" }
        },
    }
}

fn open_dive(state: AppState, id: u32) {
    if (state.log)().dive_by_id(id).is_none() {
        return;
    }
    let mut selected = state.selected;
    let mut show_trip = state.show_trip;
    let mut mobile_detail = state.mobile_detail;
    selected.set(Some(id));
    show_trip.set(None);
    mobile_detail.set(true);
}

#[derive(Debug, PartialEq)]
enum Part {
    Text(String),
    Bold(String),
    Dive { label: String, id: u32 },
}

/// Split assistant text into plain runs, `**bold**` runs and
/// `[label](dive:ID)` links. Anything else is shown verbatim.
fn rich_text(text: &str) -> Vec<Part> {
    let mut parts = Vec::new();
    let mut plain = String::new();
    let mut rest = text;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("**") {
            if let Some(end) = after.find("**") {
                if !plain.is_empty() {
                    parts.push(Part::Text(std::mem::take(&mut plain)));
                }
                parts.push(Part::Bold(after[..end].to_string()));
                rest = &after[end + 2..];
                continue;
            }
        }
        if let Some(after) = rest.strip_prefix('[') {
            if let Some((label, tail)) = after.split_once("](dive:") {
                if let Some((id, tail)) = tail.split_once(')') {
                    if let (false, Ok(id)) = (label.contains(']'), id.trim().parse::<u32>()) {
                        if !plain.is_empty() {
                            parts.push(Part::Text(std::mem::take(&mut plain)));
                        }
                        parts.push(Part::Dive {
                            label: label.to_string(),
                            id,
                        });
                        rest = tail;
                        continue;
                    }
                }
            }
        }
        let mut chars = rest.chars();
        if let Some(c) = chars.next() {
            plain.push(c);
        }
        rest = chars.as_str();
    }
    if !plain.is_empty() {
        parts.push(Part::Text(plain));
    }
    parts
}

#[component]
fn AssistantSettings(config: AiConfig, on_save: EventHandler<AiConfig>) -> Element {
    let state = use_context::<AppState>();
    let tr = state.strings();
    let mut form = use_signal(|| config.clone());
    let current = form();
    let provider_index = AiProvider::ALL
        .iter()
        .position(|provider| *provider == current.provider)
        .unwrap_or(0);

    rsx! {
        div { class: "assistant-body assistant-settings",
            p { class: "muted", "{tr.ai_setup_intro}" }
            div { class: "edit-form",
                label { class: "field-label full", "{tr.ai_provider}"
                    select {
                        class: "field",
                        value: "{provider_index}",
                        onchange: move |evt| {
                            let index = evt.value().parse::<usize>().unwrap_or(0);
                            if let Some(provider) = AiProvider::ALL.get(index).copied() {
                                let mut cfg = form.write();
                                cfg.provider = provider;
                                cfg.model = provider.default_model().to_string();
                            }
                        },
                        for (index, provider) in AiProvider::ALL.iter().enumerate() {
                            option {
                                key: "{index}",
                                value: "{index}",
                                selected: current.provider == *provider,
                                "{provider.label()}"
                            }
                        }
                    }
                }
                label { class: "field-label full", "{tr.ai_model}"
                    input {
                        class: "field",
                        value: "{current.model}",
                        oninput: move |evt| form.write().model = evt.value(),
                    }
                }
                label { class: "field-label full", "{tr.ai_api_key}"
                    input {
                        class: "field",
                        r#type: "password",
                        autocomplete: "off",
                        placeholder: "{tr.ai_api_key_placeholder}",
                        value: "{current.api_key}",
                        oninput: move |evt| form.write().api_key = evt.value(),
                    }
                }
            }
            p { class: "muted small-note", "{tr.ai_privacy_note}" }
            p { class: "muted small-note", "{tr.ai_cors_note}" }
            div { class: "detail-actions",
                button {
                    class: "btn primary",
                    r#type: "button",
                    disabled: current.api_key.trim().is_empty(),
                    onclick: move |_| on_save.call(form()),
                    "{tr.ai_save}"
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{rich_text, Part};

    #[test]
    fn parses_dive_links_and_bold() {
        assert_eq!(
            rich_text("Your **deepest** was [dive #7](dive:12)."),
            vec![
                Part::Text("Your ".into()),
                Part::Bold("deepest".into()),
                Part::Text(" was ".into()),
                Part::Dive {
                    label: "dive #7".into(),
                    id: 12
                },
                Part::Text(".".into()),
            ]
        );
        // Unclosed or non-dive markup stays as text.
        assert_eq!(
            rich_text("a **b [x](http://y) [z](dive:no)"),
            vec![Part::Text("a **b [x](http://y) [z](dive:no)".into())]
        );
    }
}
