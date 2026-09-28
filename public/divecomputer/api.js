/*
 * benthic — web dive-computer API.
 *
 * Loaded on demand by the app (`window.benthicLoadDc`). Pulls the Emscripten
 * shim in lazily and exposes a small JSON-in/JSON-out surface on
 * globalThis.benthicWeb, so the Rust side never touches the Emscripten glue.
 */

import { createSerialHost, webSerialSupported } from "./host.js";

let modulePromise = null;
let host = null;

async function ensure() {
  if (!modulePromise) {
    modulePromise = import("./benthic-dc.js").then((mod) => mod.default());
  }
  const module = await modulePromise;
  if (!host) {
    host = createSerialHost();
    globalThis.benthicHost = host;
  }
  return module;
}

function takeString(module, ptr) {
  const text = module.UTF8ToString(ptr);
  module.ccall("benthic_dc_free", null, ["number"], [ptr]);
  return text;
}

globalThis.benthicWeb = {
  supported: webSerialSupported,

  async descriptorsJson() {
    const module = await ensure();
    return takeString(module, module.ccall("benthic_dc_descriptors", "number", []));
  },

  async downloadJson(vendor, product, fingerprintHex) {
    const module = await ensure();
    return new Promise((resolve) => {
      host.result = resolve;
      module.ccall(
        "benthic_dc_download",
        null,
        ["string", "string", "string"],
        [vendor, product, fingerprintHex ?? ""],
      );
    });
  },

  async requestPort() {
    await ensure();
    try {
      await host.requestPort();
      return true;
    } catch (error) {
      return false;
    }
  },
};

export default globalThis.benthicWeb;
