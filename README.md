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

The easiest path is the checked-in [dev container](.devcontainer/), which
provides Rust, the [Dioxus CLI](https://dioxuslabs.com/learn/0.7/getting_started)
and every system library the desktop build needs. Open the folder in
VS Code ("Reopen in Container") or run `devcontainer up --workspace-folder .`.

Building natively instead requires stable Rust, the Dioxus CLI
(`cargo install dioxus-cli`) and the Linux desktop libraries; on Debian/Ubuntu:

```bash
sudo apt-get install libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev \
  libjavascriptcoregtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev \
  libxdo-dev libssl-dev pkg-config
```

On openSUSE Tumbleweed the equivalents are `webkitgtk3-devel`, `gtk3-devel`,
`libsoup-devel`, `at-spi2-core-devel`, `cairo-devel`, `pango-devel`,
`gdk-pixbuf-devel`, `glib2-devel`, `libayatana-appindicator3-devel`,
`xdotool-devel` and `libopenssl-devel`.

The native dive-computer integration (`crates/benthic-divecomputer`) builds the
vendored libdivecomputer, so it also needs `autoconf`, `automake`, `libtool`,
`pkg-config` and the libusb-1.0 and bluez development headers
(`libusb-1.0-0-dev libbluetooth-dev libudev-dev` on Debian/Ubuntu;
`libusb-1_0-devel bluez-devel libudev-devel` on openSUSE).

```bash
dx serve                      # web app at http://localhost:8080
dx serve --platform desktop   # desktop app
```

Import `dives/demo.ssrf` to see the app with data.

## Building and testing

```bash
cargo test                                   # domain model + file formats
cargo test -p benthic-divecomputer --features native  # libdivecomputer parsing
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
├── crates/benthic-divecomputer/  # native-only libdivecomputer wrapper
├── vendor/libdivecomputer/  # vendored C library (git submodule)
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
