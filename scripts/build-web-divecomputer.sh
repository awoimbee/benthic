#!/usr/bin/env bash
# Build the libdivecomputer wasm shim used by the web app.
#
# Requires emscripten (emcc/emconfigure/emmake), autoconf, automake and
# libtool. Output lands in public/divecomputer/ and is copied into the web
# build by dioxus.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
source_dir="$root/vendor/libdivecomputer"
out_dir="$root/public/divecomputer"

if ! command -v emcc >/dev/null 2>&1; then
  echo "error: emcc not found; install emscripten" >&2
  exit 1
fi
if [ ! -f "$source_dir/configure.ac" ]; then
  echo "error: vendored libdivecomputer missing; run 'git submodule update --init'" >&2
  exit 1
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$out_dir"

cp -a "$source_dir" "$work/libdc"
# Emscripten defines TIOCGSERIAL/TIOCSSERIAL but ships no <linux/serial.h>, so
# the POSIX serial backend needs the header guard tightened. The web build uses
# the custom iostream, never this code path.
sed -i '/TIOCGSERIAL.*TIOCSSERIAL/s/$/ \&\& defined(HAVE_LINUX_SERIAL_H)/' \
  "$work/libdc/src/serial_posix.c"

cd "$work/libdc"
if [ ! -x configure ]; then
  if ! autoreconf --install >"$work/autoreconf.log" 2>&1; then
    echo "error: autoreconf failed" >&2
    tail -n 40 "$work/autoreconf.log" >&2
    exit 1
  fi
fi
if ! emconfigure ./configure --host=wasm32-unknown-emscripten --disable-shared --enable-static >"$work/configure.log" 2>&1; then
  echo "error: configure failed" >&2
  tail -n 60 "$work/configure.log" >&2
  exit 1
fi
if ! emmake make -j"$(nproc)" >"$work/make.log" 2>&1; then
  echo "error: make failed" >&2
  tail -n 80 "$work/make.log" >&2
  exit 1
fi

if ! emcc "$root/web/divecomputer/shim.c" \
  -I "$work/libdc/include" \
  -L "$work/libdc/src/.libs" -ldivecomputer \
  -O2 \
  -sASYNCIFY=1 \
  -sASYNCIFY_IMPORTS='["benthic_js_configure","benthic_js_read","benthic_js_write","benthic_js_poll","benthic_js_sleep","benthic_js_close","benthic_js_ble_ioctl"]' \
  -sALLOW_MEMORY_GROWTH=1 \
  -sMODULARIZE=1 \
  -sEXPORT_ES6=1 \
  -sENVIRONMENT=web,node \
  -sEXPORTED_FUNCTIONS='["_benthic_dc_descriptors","_benthic_dc_parse","_benthic_dc_download","_benthic_dc_selftest","_benthic_dc_free","_malloc","_free"]' \
  -sEXPORTED_RUNTIME_METHODS='["ccall","cwrap","UTF8ToString","lengthBytesUTF8","stringToUTF8"]' \
  -o "$out_dir/benthic-dc.mjs" >"$work/emcc.log" 2>&1; then
  echo "error: emcc failed" >&2
  tail -n 60 "$work/emcc.log" >&2
  exit 1
fi

echo "built $out_dir/benthic-dc.mjs"
