use std::env;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

mod shim;

const HELP: &str = "\
Development tasks for benthic.

Usage: cargo xtask <task> [args]

Tasks:
  check                  Run all checks (same as CI): fmt-check, clippy, test, check-web
  fmt                    Format the workspace
  fmt-check              Check formatting
  clippy                 Lint the workspace with warnings denied
  test                   Run the workspace tests
  check-web              Type-check the web app for wasm32
  check-native           Build and test the native libdivecomputer integration
                         (needs the devcontainer, or autoconf, automake, libtool,
                         pkg-config plus the libusb-1.0 and bluez headers)
  serve                  Serve the web app at http://localhost:8080 (builds the
                         dive-computer shim first if it is missing or stale)
  serve-desktop          Serve the desktop app
  build-web-shim         Build the libdivecomputer wasm shim into public/divecomputer
                         (uses emcc from PATH, or installs emsdk under target/)
  build-web [BASE_PATH]  Production web build for GitHub Pages (default: /benthic)
  parse <PATH>           Parse a dive log from the command line
  clean                  Remove build artifacts
  help                   Show this message
";

pub(crate) type Result = std::result::Result<(), String>;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let task = args.first().map(String::as_str).unwrap_or("help");
    let rest = &args[args.len().min(1)..];

    let result = match task {
        "check" => check(),
        "fmt" => cargo(["fmt", "--all"]),
        "fmt-check" => fmt_check(),
        "clippy" => clippy(),
        "test" => test(),
        "check-web" => check_web(),
        "check-native" => cargo(["test", "-p", "benthic-divecomputer", "--features", "native"]),
        "serve" => shim::ensure().and_then(|()| run("dx", ["serve"])),
        "serve-desktop" => run("dx", ["serve", "--platform", "desktop"]),
        "build-web-shim" => shim::build(),
        "build-web" => build_web(rest.first().map(String::as_str).unwrap_or("/benthic")),
        "parse" => match rest.first() {
            Some(path) => cargo([
                "run",
                "-q",
                "-p",
                "benthic-core",
                "--example",
                "parse_ssrf",
                "--",
                path,
            ]),
            None => Err("usage: cargo xtask parse <PATH>".into()),
        },
        "clean" => cargo(["clean"]),
        "help" | "-h" | "--help" => {
            print!("{HELP}");
            Ok(())
        }
        other => Err(format!("unknown task `{other}`\n\n{HELP}")),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn check() -> Result {
    fmt_check()?;
    clippy()?;
    test()?;
    check_web()
}

fn fmt_check() -> Result {
    cargo(["fmt", "--all", "--", "--check"])
}

fn clippy() -> Result {
    cargo([
        "clippy",
        "--workspace",
        "--all-targets",
        "--",
        "-D",
        "warnings",
    ])
}

fn test() -> Result {
    cargo(["test", "--workspace"])
}

fn check_web() -> Result {
    cargo(["check", "--target", "wasm32-unknown-unknown"])
}

fn build_web(base_path: &str) -> Result {
    shim::build()?;
    run(
        "dx",
        [
            "build",
            "--release",
            "--platform",
            "web",
            "--base-path",
            base_path,
        ],
    )
}

pub(crate) fn project_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives in the workspace root")
        .to_path_buf()
}

fn cargo<I, S>(args: I) -> Result
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    run(cargo, args)
}

pub(crate) fn run<P, I, S>(program: P, args: I) -> Result
where
    P: AsRef<OsStr>,
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let program = program.as_ref();
    let mut cmd = Command::new(program);
    cmd.args(args).current_dir(project_root());
    let display = std::iter::once(program)
        .chain(cmd.get_args())
        .map(OsStr::to_string_lossy)
        .collect::<Vec<_>>()
        .join(" ");
    eprintln!("$ {display}");
    let status = cmd
        .status()
        .map_err(|err| format!("failed to run `{display}`: {err}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`{display}` failed with {status}"))
    }
}
