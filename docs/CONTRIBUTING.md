# Contributing to benthic

Thanks for helping! This project is an independent Rust/Dioxus rewrite of
Subsurface. The most valuable contributions right now are importers, format
fidelity fixes, and UI for the Phase 1 features in [ROADMAP.md](../ROADMAP.md).

## Getting set up

The repo ships a [dev container](../.devcontainer/) with Rust, the Dioxus CLI
and all the desktop system libraries, so no host setup is required. Open the
folder in a dev-container-aware editor (VS Code: "Reopen in Container") or run
`devcontainer up --workspace-folder .`.

To set up a host manually:

```bash
git clone https://github.com/awoimbee/benthic
cd benthic
rustup target add wasm32-unknown-unknown
cargo install dioxus-cli          # if you don't have dx

cargo test                        # run the core test suite
dx serve                          # web app at localhost:8080
dx serve --platform desktop       # desktop app
```

See [README.md](../README.md#quick-start) for the native Linux system packages
needed by the desktop build.

Import `dives/demo.ssrf` from the app to get sample data.

## Before opening a PR

Run the same checks CI runs:

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
cargo check --target wasm32-unknown-unknown
```

## Where code goes

* **Domain logic, formats, math → `crates/benthic-core`.** It must not depend
  on Dioxus and must compile for `wasm32` and native without `cfg` hacks.
* **UI, state, platform glue → the root `benthic` crate.**

Add tests with any behavior change. Format work must include a round-trip test
(prefer a checked-in fixture over an inline blob once it grows).

## Commit conventions

* Keep commits small and focused; explain *why*, not just *what*.
* Use an imperative subject line (`ssrf: preserve sensor pressure ids`).
* Prefix with the area when it helps (`core:`, `ui:`, `ci:`, `docs:`).
* No emojis in commit messages, code comments or PR descriptions.

## Code style

* `cargo fmt` is authoritative.
* Prefer explicit integer units over `f64` for physical quantities.
* Avoid `unwrap()` in library code on user-controlled input; return `Result`
  or fall back to a documented default.
* Keep `benthic-core` free of platform code.

## Licensing and AI assistance

* benthic is **GPL-2.0-or-later**. By contributing you agree to license your
  work under the same terms.
* If you use an AI assistant to write a non-trivial block, add a short
  `// AI-generated` comment so reviewers can calibrate their review. This
  mirrors Subsurface's own policy.

## Reporting issues

Include: what you did, what happened, what you expected, and (for format bugs)
the smallest dive log that reproduces it. Please redact personal data.
