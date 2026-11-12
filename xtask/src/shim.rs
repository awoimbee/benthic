//! Builds the libdivecomputer wasm shim used by the web app.
//!
//! libdivecomputer is compiled straight from the vendored sources with `emcc`;
//! the target is always wasm32-emscripten, so the autotools `config.h` is fixed
//! and written here instead of running `configure`. `emcc` is taken from `PATH`
//! when available, otherwise a pinned emsdk is installed under `target/emsdk`.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::SystemTime;

use crate::{project_root, run, Result};

const EMSDK_VERSION: &str = "3.1.69";
const EMSDK_REPO: &str = "https://github.com/emscripten-core/emsdk.git";

/// The `config.h` that libdivecomputer's `configure` produces for
/// `--host=wasm32-unknown-emscripten` (minus the version, added below).
const CONFIG_H: &str = "\
#define ENABLE_LOGGING 1
#define HAVE_CLOCK_GETTIME 1
#define HAVE_DECL_OPTRESET 1
#define HAVE_DECL_STRERROR_R 1
#define HAVE_GETOPT_H 1
#define HAVE_GETOPT_LONG 1
#define HAVE_GMTIME_R 1
#define HAVE_LOCALTIME_R 1
#define HAVE_PTHREAD_H 1
#define HAVE_STRERROR_R 1
#define HAVE_STRUCT_TM_TM_GMTOFF 1
#define HAVE_SYS_PARAM_H 1
#define HAVE_SYS_SOCKET_H 1
#define HAVE_TIMEGM 1
#define HAVE_UNISTD_H 1
";

const CFLAGS: &[&str] = &[
    "-O2",
    "-DHAVE_CONFIG_H",
    "-Wall",
    "-Wformat=2",
    "-Wwrite-strings",
    "-Wpointer-arith",
    "-Wstrict-prototypes",
    "-Wno-unused-parameter",
    "-Wno-unused-function",
    "-Wno-unused-variable",
    "-Wno-unused-but-set-variable",
    "-Wno-pointer-sign",
    "-Wno-shadow",
    "-Werror=enum-conversion",
];

const ASYNCIFY_IMPORTS: &str = r#"-sASYNCIFY_IMPORTS=["benthic_js_configure","benthic_js_read","benthic_js_write","benthic_js_poll","benthic_js_sleep","benthic_js_close","benthic_js_ble_ioctl"]"#;
const EXPORTED_FUNCTIONS: &str = r#"-sEXPORTED_FUNCTIONS=["_benthic_dc_descriptors","_benthic_dc_parse","_benthic_dc_download","_benthic_dc_selftest","_benthic_dc_free","_malloc","_free"]"#;
const EXPORTED_RUNTIME_METHODS: &str = r#"-sEXPORTED_RUNTIME_METHODS=["ccall","cwrap","UTF8ToString","lengthBytesUTF8","stringToUTF8"]"#;

fn output() -> PathBuf {
    project_root().join("public/divecomputer/benthic-dc.mjs")
}

fn shim_source() -> PathBuf {
    project_root().join("web/divecomputer/shim.c")
}

fn target_dir() -> PathBuf {
    env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| project_root().join("target"))
}

/// Builds the shim unless an up-to-date one is already in `public/`.
pub fn ensure() -> Result {
    let mjs = output();
    let fresh = [mjs.clone(), mjs.with_extension("wasm")]
        .iter()
        .all(|out| modified(out).is_some_and(|built| Some(built) >= modified(&shim_source())));
    if fresh {
        return Ok(());
    }
    eprintln!("web dive-computer shim missing or stale, building it");
    build()
}

pub fn build() -> Result {
    let root = project_root();
    let libdc = root.join("vendor/libdivecomputer");
    if !libdc.join("configure.ac").is_file() {
        return Err(
            "vendored libdivecomputer missing; run `git submodule update --init --recursive`"
                .into(),
        );
    }
    let emcc = emcc()?;

    let work = target_dir().join("divecomputer-wasm");
    if work.exists() {
        fs::remove_dir_all(&work).map_err(|e| format!("clean {}: {e}", work.display()))?;
    }
    let generated = work.join("include");
    let objects = work.join("obj");
    create_dir(&generated.join("libdivecomputer"))?;
    create_dir(&objects)?;
    generate_headers(&libdc, &generated)?;

    let sources = sources(&libdc, &work)?;
    let includes = [generated.clone(), libdc.join("include"), libdc.join("src")];
    compile_all(&emcc, &sources, &includes, &objects)?;

    let out = output();
    create_dir(out.parent().unwrap())?;
    let mut args: Vec<String> = vec![
        shim_source().display().to_string(),
        "-I".into(),
        generated.display().to_string(),
        "-I".into(),
        libdc.join("include").display().to_string(),
    ];
    let mut objs: Vec<String> = fs::read_dir(&objects)
        .map_err(|e| format!("read {}: {e}", objects.display()))?
        .map(|entry| entry.map(|e| e.path().display().to_string()))
        .collect::<std::io::Result<_>>()
        .map_err(|e| e.to_string())?;
    objs.sort();
    args.extend(objs);
    args.extend(
        [
            "-O2",
            "-sASYNCIFY=1",
            ASYNCIFY_IMPORTS,
            "-sALLOW_MEMORY_GROWTH=1",
            "-sMODULARIZE=1",
            "-sEXPORT_ES6=1",
            "-sENVIRONMENT=web,node",
            EXPORTED_FUNCTIONS,
            EXPORTED_RUNTIME_METHODS,
            "-o",
        ]
        .map(String::from),
    );
    args.push(out.display().to_string());
    eprintln!("linking {}", out.display());
    quiet(Command::new(&emcc).args(&args), "emcc (link)")?;
    eprintln!("built {}", out.display());
    Ok(())
}

/// `version.h` and `revision.h`, normally produced by `configure` and `make`.
fn generate_headers(libdc: &Path, generated: &Path) -> Result {
    let configure = read(&libdc.join("configure.ac"))?;
    let m4 = |name: &str| -> String {
        let key = format!("m4_define([{name}],[");
        configure
            .find(&key)
            .and_then(|start| {
                let rest = &configure[start + key.len()..];
                rest.find(']').map(|end| rest[..end].to_string())
            })
            .unwrap_or_default()
    };
    let (major, minor, micro) = (
        m4("dc_version_major"),
        m4("dc_version_minor"),
        m4("dc_version_micro"),
    );
    let suffix = m4("dc_version_suffix");
    let mut version = format!("{major}.{minor}.{micro}");
    if !suffix.is_empty() {
        version = format!("{version}-{suffix}");
    }

    let template = read(&libdc.join("include/libdivecomputer/version.h.in"))?;
    let version_h = template
        .replace("@DC_VERSION@", &version)
        .replace("@DC_VERSION_MAJOR@", &major)
        .replace("@DC_VERSION_MINOR@", &minor)
        .replace("@DC_VERSION_MICRO@", &micro);
    write(&generated.join("libdivecomputer/version.h"), &version_h)?;

    let mut config = format!("{CONFIG_H}#define VERSION \"{version}\"\n");
    if !suffix.is_empty() {
        config.push_str("#define HAVE_VERSION_SUFFIX 1\n");
    }
    write(&generated.join("config.h"), &config)?;

    let revision = Command::new("git")
        .args(["rev-parse", "--verify", "HEAD"])
        .current_dir(libdc)
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .unwrap_or_default();
    write(
        &generated.join("revision.h"),
        &format!("#define DC_VERSION_REVISION \"{revision}\"\n"),
    )
}

/// The library sources. Emscripten defines `TIOCGSERIAL`/`TIOCSSERIAL` but has
/// no `<linux/serial.h>`, so the POSIX serial backend gets its guard tightened;
/// the web build only ever uses the custom iostream.
fn sources(libdc: &Path, work: &Path) -> std::result::Result<Vec<PathBuf>, String> {
    let src = libdc.join("src");
    let mut sources = Vec::new();
    for entry in fs::read_dir(&src).map_err(|e| format!("read {}: {e}", src.display()))? {
        let path = entry.map_err(|e| e.to_string())?.path();
        let name = path.file_name().unwrap().to_string_lossy();
        if path.extension().is_none_or(|ext| ext != "c") || name == "serial_win32.c" {
            continue;
        }
        if name == "serial_posix.c" {
            let guard = "#if defined(TIOCGSERIAL) && defined(TIOCSSERIAL)";
            let patched = read(&path)?.replace(
                guard,
                "#if defined(TIOCGSERIAL) && defined(TIOCSSERIAL) && defined(HAVE_LINUX_SERIAL_H)",
            );
            let copy = work.join("serial_posix.c");
            write(&copy, &patched)?;
            sources.push(copy);
        } else {
            sources.push(path);
        }
    }
    sources.sort();
    Ok(sources)
}

fn compile_all(emcc: &Path, sources: &[PathBuf], includes: &[PathBuf], objects: &Path) -> Result {
    eprintln!("compiling {} libdivecomputer sources", sources.len());
    let next = AtomicUsize::new(0);
    let errors = Mutex::new(Vec::new());
    let jobs = std::thread::available_parallelism().map_or(4, |n| n.get());
    std::thread::scope(|scope| {
        for _ in 0..jobs {
            scope.spawn(|| {
                while let Some(source) = sources.get(next.fetch_add(1, Ordering::Relaxed)) {
                    let stem = source.file_stem().unwrap().to_string_lossy();
                    let mut cmd = Command::new(emcc);
                    cmd.args(CFLAGS);
                    for dir in includes {
                        cmd.arg("-I").arg(dir);
                    }
                    cmd.arg("-c")
                        .arg(source)
                        .arg("-o")
                        .arg(objects.join(format!("{stem}.o")));
                    if let Err(err) = quiet(&mut cmd, &format!("emcc {}", source.display())) {
                        errors.lock().unwrap().push(err);
                    }
                }
            });
        }
    });
    let errors = errors.into_inner().unwrap();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
}

/// Runs a command, only showing its output when it fails.
fn quiet(cmd: &mut Command, what: &str) -> Result {
    let out = cmd
        .output()
        .map_err(|e| format!("failed to run {what}: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    Err(format!(
        "{what} failed with {}\n{}{}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    ))
}

/// `emcc` from `PATH`, or from a pinned emsdk installed on first use.
fn emcc() -> std::result::Result<PathBuf, String> {
    let name = if cfg!(windows) { "emcc.bat" } else { "emcc" };
    if let Some(found) = env::var_os("PATH")
        .iter()
        .flat_map(env::split_paths)
        .map(|dir| dir.join(name))
        .find(|path| path.is_file())
    {
        return Ok(found);
    }

    let emsdk = target_dir().join("emsdk");
    let marker = emsdk.join(format!(".benthic-{EMSDK_VERSION}"));
    let emcc = emsdk.join("upstream/emscripten").join(name);
    if !(marker.is_file() && emcc.is_file()) {
        install_emsdk(&emsdk, &marker)?;
    }
    Ok(emcc)
}

fn install_emsdk(emsdk: &Path, marker: &Path) -> Result {
    eprintln!(
        "emcc not found on PATH, installing emsdk {EMSDK_VERSION} into {}",
        emsdk.display()
    );
    if emsdk.exists() {
        fs::remove_dir_all(emsdk).map_err(|e| format!("clean {}: {e}", emsdk.display()))?;
    }
    let dest = emsdk.display().to_string();
    run(
        "git",
        [
            "clone",
            "--quiet",
            "--depth",
            "1",
            "--branch",
            EMSDK_VERSION,
            EMSDK_REPO,
            &dest,
        ],
    )?;
    let tool = emsdk.join(if cfg!(windows) { "emsdk.bat" } else { "emsdk" });
    run(&tool, ["install", EMSDK_VERSION])?;
    run(&tool, ["activate", EMSDK_VERSION])?;
    write(marker, "")
}

fn modified(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn create_dir(path: &Path) -> Result {
    fs::create_dir_all(path).map_err(|e| format!("create {}: {e}", path.display()))
}

fn read(path: &Path) -> std::result::Result<String, String> {
    fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))
}

fn write(path: &Path, contents: &str) -> Result {
    fs::write(path, contents).map_err(|e| format!("write {}: {e}", path.display()))
}
