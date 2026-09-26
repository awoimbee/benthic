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

# Serve the web app at http://localhost:8080
serve:
    dx serve

# Serve the desktop app
serve-desktop:
    dx serve --platform desktop

# Production web build, suitable for GitHub Pages
build-web base_path="/benthic":
    dx build --release --platform web --base-path "{{base_path}}"

# Parse a dive log from the command line (developer utility)
parse path:
    cargo run -q -p benthic-core --example parse_ssrf -- {{path}}

clean:
    cargo clean
