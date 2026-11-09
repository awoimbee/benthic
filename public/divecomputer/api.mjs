/*
 * benthic — web dive-computer API.
 *
 * Loaded on demand by the app (`window.benthicLoadDc`). Pulls the Emscripten
 * shim in lazily and exposes a small JSON-in/JSON-out surface on
 * globalThis.benthicWeb, so the Rust side never touches the Emscripten glue.
 *
 * Transports use libdivecomputer bits: 1 is Web Serial, 32 (1 << 5) is
 * Web Bluetooth. The shim's download entry point still takes its historical
 * 0/1 transport index, so the adapter translates before calling into C.
 *
 * Everything under public/divecomputer/ is prebuilt and shipped with a `.mjs`
 * extension on purpose: the Dioxus bundler only runs esbuild over `.js` assets,
 * and the Emscripten glue contains Node-only imports it cannot resolve. `.mjs`
 * assets are treated as opaque files and copied verbatim.
 */

import {
  createBleHost,
  createSerialHost,
  webBluetoothSupported,
  webSerialSupported,
} from "./host.mjs";

const SERIAL = 1 << 0;
const BLUETOOTH = 1 << 5;
const SHIM_SERIAL = 0;
const SHIM_BLUETOOTH = 1;

function shimTransport(transport) {
  return transport === BLUETOOTH ? SHIM_BLUETOOTH : SHIM_SERIAL;
}

let modulePromise = null;
let serialHost = null;
let bleHost = null;

async function loadModule() {
  if (!modulePromise) {
    modulePromise = import("./benthic-dc.mjs").then((mod) => mod.default());
  }
  return modulePromise;
}

async function useHost(transport) {
  const module = await loadModule();
  if (transport === BLUETOOTH) {
    if (!bleHost) bleHost = createBleHost();
    globalThis.benthicHost = bleHost;
  } else {
    if (!serialHost) serialHost = createSerialHost();
    globalThis.benthicHost = serialHost;
  }
  return module;
}

function takeString(module, ptr) {
  const text = module.UTF8ToString(ptr);
  module.ccall("benthic_dc_free", null, ["number"], [ptr]);
  return text;
}

globalThis.benthicWeb = {
  serialSupported: webSerialSupported,
  // Backwards-compatible alias used by older wasm builds.
  supported: webSerialSupported,
  bluetoothSupported: webBluetoothSupported,

  async descriptorsJson() {
    const module = await loadModule();
    return takeString(module, module.ccall("benthic_dc_descriptors", "number", []));
  },

  async connect(transport) {
    const module = await useHost(transport);
    void module;
    const host = globalThis.benthicHost;
    try {
      if (transport === BLUETOOTH) {
        await host.requestDevice();
      } else {
        await host.requestPort();
      }
      return true;
    } catch (error) {
      return false;
    }
  },

  async downloadJson(vendor, product, transport, fingerprintHex, eventCallback) {
    const module = await useHost(transport);
    const host = globalThis.benthicHost;
    const previousEvent = host.event;
    host.event = (event) => {
      try {
        eventCallback?.(event);
      } catch {
        /* progress display must never break the download */
      }
    };
    try {
      return await new Promise((resolve) => {
        host.result = resolve;
        module.ccall(
          "benthic_dc_download",
          null,
          ["string", "string", "number", "string"],
          [vendor, product, shimTransport(transport), fingerprintHex ?? ""],
        );
      });
    } finally {
      host.event = previousEvent ?? (() => {});
    }
  },
};

export default globalThis.benthicWeb;
