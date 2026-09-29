/*
 * benthic — web dive-computer API.
 *
 * Loaded on demand by the app (`window.benthicLoadDc`). Pulls the Emscripten
 * shim in lazily and exposes a small JSON-in/JSON-out surface on
 * globalThis.benthicWeb, so the Rust side never touches the Emscripten glue.
 *
 * Transport 0 is Web Serial, 1 is Web Bluetooth.
 */

import {
  createBleHost,
  createSerialHost,
  webBluetoothSupported,
  webSerialSupported,
} from "./host.js";

const SERIAL = 0;
const BLUETOOTH = 1;

let modulePromise = null;
let serialHost = null;
let bleHost = null;

async function loadModule() {
  if (!modulePromise) {
    modulePromise = import("./benthic-dc.js").then((mod) => mod.default());
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

  async downloadJson(vendor, product, transport, fingerprintHex) {
    const module = await useHost(transport);
    return new Promise((resolve) => {
      globalThis.benthicHost.result = resolve;
      module.ccall(
        "benthic_dc_download",
        null,
        ["string", "string", "number", "string"],
        [vendor, product, transport, fingerprintHex ?? ""],
      );
    });
  },
};

export default globalThis.benthicWeb;
