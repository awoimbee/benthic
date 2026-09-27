# benthic

A modern, cross-platform dive log — a ground-up rewrite of
[Subsurface](https://subsurface-divelog.org/) in Rust and
[Dioxus](https://dioxuslabs.com/).

* Runs in the browser and on the desktop from one Rust/Dioxus codebase.
* Local-first: no account, no server, automatic persistence, explicit
  import/export.
* Reads and writes Subsurface-compatible XML (`.ssrf`).
* Deployed from `main` to <https://awoimbee.github.io/benthic/>.

See [ROADMAP.md](ROADMAP.md) for status and what's next.

## Quick start

Requires stable Rust and the
[Dioxus CLI](https://dioxuslabs.com/learn/0.7/getting_started)
(`cargo install dioxus-cli`); the Linux desktop build also needs
`webkit2gtk-4.1`.

```bash
dx serve                      # web app at http://localhost:8080
dx serve --platform desktop   # desktop app
```

Import `dives/demo.ssrf` to see the app with data.

## Building and testing

```bash
cargo test                                   # domain model + file formats
cargo check --target wasm32-unknown-unknown  # type-check the web app
cargo fmt --all
cargo clippy --all-targets -- -D warnings

dx build --release --platform web --base-path /benthic
# output: target/dx/benthic/release/web/public
```

Drop `--base-path` (or use `/`) when serving from a domain root.

## Project layout

```
benthic/
├── crates/benthic-core/   # domain model + file formats (no UI, no platform)
├── src/                   # Dioxus app: root component, state, storage, components
├── assets/main.css
├── public/                # static web assets
├── dives/demo.ssrf        # demo log used by the app and tests
└── docs/                  # architecture, data model, formats, contributing
```

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) and
[ROADMAP.md](ROADMAP.md).

## License

GPL-2.0-or-later, matching Subsurface. See [LICENSE](LICENSE).
