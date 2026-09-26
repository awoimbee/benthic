# benthic

A modern, cross-platform dive log — a ground-up rewrite of
[Subsurface](https://subsurface-divelog.org/) in Rust and
[Dioxus](https://dioxuslabs.com/).

* **One codebase, many targets.** The same Rust/Dioxus app runs in the browser
  and on the desktop (and, later, mobile).
* **Local-first.** Your log lives on your device. There is no account and no
  server. Persistence is automatic, and explicit import/export keeps your data
  portable.
* **Compatible by design.** benthic reads and writes Subsurface-compatible XML
  (`.ssrf`) so you can move between the two without losing your history.
* **Web build on GitHub Pages.** `main` is continuously deployed to
  <https://awoimbee.github.io/benthic/>.

> **Status.** Phase 1 is complete. Phase 2 is complete except VPM-B, which is
> deliberately deferred (see [ROADMAP.md](ROADMAP.md)): a faithful port of
> Subsurface's equations produced schedules that did not match reference
> behaviour and could not be validated here, and an unvalidated
> decompression model is not safe to ship. The log is usable end to end:
> import, create/edit, search, filter and group dives, equipment editing,
> unit and format preferences, undo/redo, a command palette, automatic
> backups, an interactive dive profile, and a validated Bühlmann dive
> planner (open circuit, CCR and pSCR) with gas needs and bailout.

## Quick start

Prerequisites:

* Rust **stable** (a `rust-toolchain.toml` pins stable + the wasm target)
* The [Dioxus CLI](https://dioxuslabs.com/learn/0.7/getting_started): `cargo install dioxus-cli`
* For the desktop build on Linux: `webkit2gtk-4.1` development packages

Run the web app locally:

```bash
dx serve            # serves at http://localhost:8080
```

Run the desktop app:

```bash
dx serve --platform desktop
# or, without the Dioxus CLI:
cargo run --no-default-features --features desktop
```

Import the bundled demo log (`dives/demo.ssrf`) with the **Import** button to
see the app with data.

## Building and testing

```bash
cargo test                                   # domain model + file formats
cargo test -p benthic-core                   # core only
cargo check --target wasm32-unknown-unknown  # type-check the web app
cargo fmt --all
cargo clippy --all-targets -- -D warnings
```

Build a production web bundle (the same command CI uses):

```bash
dx build --release --platform web --base-path /benthic
# output: target/dx/benthic/release/web/public
```

Drop `--base-path` (or use `/`) when serving from a domain root or a custom
domain.

## Project layout

```
benthic/
├── crates/
│   └── benthic-core/        # domain model + file formats (no UI, no platform)
│       ├── src/units.rs     # strongly-typed integer physical units
│       ├── src/gas.rs       # breathing-gas mixtures
│       ├── src/model.rs     # Dive, Sample, DiveComputer, DiveSite, DiveLog, ...
│       └── src/io/          # native JSON + Subsurface XML codecs
├── src/
│   ├── main.rs              # entry point
│   ├── app.rs               # root component + autosave/load
│   ├── state.rs             # shared signals (context)
│   ├── storage.rs           # localStorage (web) / data dir (desktop)
│   ├── platform.rs          # file download/save
│   └── components/          # UI components
├── assets/main.css
├── public/                  # static web assets
├── dives/demo.ssrf          # demo log used by the app and tests
└── docs/                    # architecture, data model, formats, contributing
```

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the design and
[ROADMAP.md](ROADMAP.md) for what's next.

## License

GPL-2.0-or-later, matching Subsurface. See [LICENSE](LICENSE).
benthic is an independent reimplementation; Subsurface's data formats are
documented in [docs/FILE-FORMATS.md](docs/FILE-FORMATS.md).
