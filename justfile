# Common development tasks. Install with: https://github.com/casey/just
#
#   just            # list recipes
#   just check      # everything CI runs

set shell := ["bash", "-uc"]

# Run all checks (same as CI)
check: fmt-check clippy test check-web

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all -- --check

clippy:
    cargo clippy --workspace --all-targets -- -D warnings

test:
    cargo test --workspace

check-web:
    cargo check --target wasm32-unknown-unknown

# Build and test the native libdivecomputer integration. Requires the
# devcontainer (or autoconf, automake, libtool, pkg-config plus the libusb-1.0
# and bluez development headers).
check-native:
    cargo test -p benthic-divecomputer --features native

# Serve the web app at http://localhost:8080
serve:
    dx serve

# Serve the desktop app
serve-desktop:
    dx serve --platform desktop

# Build the libdivecomputer wasm shim into public/divecomputer (needs emscripten).
build-web-shim:
    bash scripts/build-web-divecomputer.sh

# Production web build, suitable for GitHub Pages
build-web base_path="/benthic": build-web-shim
    dx build --release --platform web --base-path "{{base_path}}"

# Parse a dive log from the command line (developer utility)
parse path:
    cargo run -q -p benthic-core --example parse_ssrf -- {{path}}

clean:
    cargo clean
