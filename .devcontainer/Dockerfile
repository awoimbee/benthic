# Development container for benthic.
#
# Bakes in everything needed to build both targets from one image:
#   * the Dioxus CLI (`dx`)
#   * the wasm32 target for the web build
#   * the Linux desktop system libraries (wry/tao -> WebKitGTK/GTK3)
#
# The base image ships Rust, rustup, cargo, git and common build tools.
FROM mcr.microsoft.com/devcontainers/rust:1-trixie

USER root

# Pin the CLI to the version CI deploys with.
ARG DIOXUS_CLI_VERSION=0.7.10

# System libraries required by the native (`--platform desktop`) build.
# These are the Debian packages behind the pkg-config lookups performed by
# the *-sys crates: webkit2gtk-sys, javascriptcore-rs-sys, soup3-sys,
# gtk/gdk/pango/gdk-pixbuf/atk-sys, libappindicator-sys, libxdo-sys and
# openssl-sys. Keep in sync with README.md.
RUN apt-get update \
    && export DEBIAN_FRONTEND=noninteractive \
    && apt-get install -y --no-install-recommends \
        build-essential \
        pkg-config \
        ca-certificates \
        curl \
        file \
        libwebkit2gtk-4.1-dev \
        libjavascriptcoregtk-4.1-dev \
        libgtk-3-dev \
        libsoup-3.0-dev \
        libayatana-appindicator3-dev \
        librsvg2-dev \
        libxdo-dev \
        libssl-dev \
        autoconf \
        automake \
        libtool \
        libusb-1.0-0-dev \
        libbluetooth-dev \
        libudev-dev \
        libftdi1-dev \
        emscripten \
    && rm -rf /var/lib/apt/lists/*

# The web app type-checks and builds for wasm32.
RUN rustup target add wasm32-unknown-unknown

# Install the prebuilt CLI (the same artifact the Pages workflow uses),
# avoiding a slow `cargo install dioxus-cli`. The release publishes one
# tarball per architecture, so pick the matching Rust target triple.
RUN arch="$(dpkg --print-architecture)"; \
    case "$arch" in \
        amd64) dx_target="x86_64-unknown-linux-gnu" ;; \
        arm64) dx_target="aarch64-unknown-linux-gnu" ;; \
        *) echo "unsupported architecture: $arch" >&2; exit 1 ;; \
    esac; \
    curl -fsSL \
        "https://github.com/DioxusLabs/dioxus/releases/download/v${DIOXUS_CLI_VERSION}/dx-${dx_target}.tar.gz" \
        | tar -xz -C /usr/local/bin; \
    dx --version
