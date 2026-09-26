#![allow(non_snake_case)]

//! benthic — a modern dive log, built with Dioxus.
//!
//! The binary is intentionally thin: all domain logic lives in `benthic-core`,
//! and this crate only wires it to a UI and to platform storage.

mod app;
mod components;
mod format;
mod platform;
mod state;
mod storage;

use app::App;

fn main() {
    dioxus::launch(App);
}
