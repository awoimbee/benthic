//! Builds the vendored libdivecomputer and links it into this crate.
//!
//! libdivecomputer ships its own autotools build system, so we copy the
//! vendored source into `OUT_DIR`, configure a static build there and install
//! it into a private prefix. Nothing needs to be installed system-wide.
//!
//! The crate is a no-op on wasm: the web build never touches this script or
//! the C library.
//!
//! Build requirements (Linux): autoconf, automake, libtool, pkg-config, a C
//! compiler and the development headers for libusb-1.0 and bluez.

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../vendor/libdivecomputer/configure.ac");
    println!("cargo:rerun-if-changed=../../vendor/libdivecomputer/include");
    println!("cargo:rerun-if-changed=../../vendor/libdivecomputer/src");

    // The web build stays pure Rust: compile to an empty crate.
    if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("wasm32") {
        return;
    }

    // The C library is only built when explicitly requested, so a bare
    // `cargo check`/`clippy`/`test` on a machine without the toolchain still
    // works (the FFI code is type-checked, just never linked).
    if std::env::var_os("CARGO_FEATURE_NATIVE").is_none() {
        return;
    }

    if !cfg!(target_os = "linux") {
        panic!("benthic-divecomputer currently supports Linux only");
    }

    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let source = manifest.join("../../vendor/libdivecomputer");
    let source = source.canonicalize().unwrap_or_else(|_| {
        panic!(
            "vendored libdivecomputer not found at {}; run \
             `git submodule update --init --recursive`",
            source.display()
        )
    });

    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let build = out.join("libdivecomputer");
    let prefix = out.join("libdc");

    if !build.join("configure").exists() {
        copy_tree(&source, &build);
        run(&build, "autoreconf", &["--install"]);
    }

    if !prefix.join("lib/libdivecomputer.a").exists() {
        run(
            &build,
            "./configure",
            &[
                &format!("--prefix={}", prefix.display()),
                "--disable-shared",
                "--enable-static",
            ],
        );
        run(&build, "make", &[&format!("-j{}", jobs())]);
        run(&build, "make", &["install"]);
    }

    println!(
        "cargo:rustc-link-search=native={}",
        prefix.join("lib").display()
    );
    println!("cargo:rustc-link-lib=static=divecomputer");
    // The private dependencies advertised by the generated libdivecomputer.pc.
    println!("cargo:rustc-link-lib=usb-1.0");
    println!("cargo:rustc-link-lib=bluetooth");
    println!("cargo:rustc-link-lib=m");
}

fn jobs() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

fn run(dir: &Path, cmd: &str, args: &[&str]) {
    let status = Command::new(cmd)
        .current_dir(dir)
        .args(args)
        .status()
        .unwrap_or_else(|e| panic!("failed to run `{cmd}`: {e}"));
    assert!(status.success(), "`{cmd} {}` failed", args.join(" "));
}

/// Recursively copy `from` into `to`, skipping the submodule's `.git` file.
fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("create build directory");
    for entry in std::fs::read_dir(from).expect("read vendored libdivecomputer") {
        let entry = entry.expect("directory entry");
        let name = entry.file_name();
        if name == ".git" {
            continue;
        }
        let src = entry.path();
        let dst = to.join(&name);
        if entry.file_type().expect("file type").is_dir() {
            copy_tree(&src, &dst);
        } else {
            std::fs::copy(&src, &dst).expect("copy file");
        }
    }
}
