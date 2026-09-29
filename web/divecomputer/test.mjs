// Smoke test for the wasm shim: enumerate descriptors and parse a fixture
// through the same neutral JSON the Rust app consumes.
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import createBenthicDc from "../../public/divecomputer/benthic-dc.js";

const here = dirname(fileURLToPath(import.meta.url));
const Module = await createBenthicDc();

function takeString(ptr) {
  const text = Module.UTF8ToString(ptr);
  Module.ccall("benthic_dc_free", null, ["number"], [ptr]);
  return text;
}

function parse(vendor, product, bytes) {
  const ptr = Module._malloc(bytes.length);
  Module.HEAPU8.set(bytes, ptr);
  const out = Module.ccall(
    "benthic_dc_parse",
    "number",
    ["string", "string", "number", "number"],
    [vendor, product, ptr, bytes.length],
  );
  Module._free(ptr);
  if (!out) return null;
  return JSON.parse(takeString(out));
}

const descriptors = JSON.parse(
  takeString(Module.ccall("benthic_dc_descriptors", "number", [])),
);
if (descriptors.length < 100) throw new Error(`only ${descriptors.length} descriptors`);

const dir = join(here, "../../vendor/libdivecomputer/test/fixtures");
const shearwater = parse(
  "Shearwater",
  "Petrel 2",
  readFileSync(join(dir, "shearwater_petrel2-0001.bin")),
);
if (!shearwater) throw new Error("parse returned null");
if (shearwater.samples.length < 3000) throw new Error("too few samples");
if (!shearwater.samples.some((s) => s.t === "depth")) throw new Error("no depth samples");
if (!shearwater.samples.some((s) => s.t === "setpoint")) throw new Error("no setpoint");
if (shearwater.gasmixes.length !== 2) throw new Error("expected 2 gas mixes");
if (!shearwater.datetime?.year) throw new Error("no datetime");

for (const [file, vendor, product] of [
  ["hw_ostc5-0001.bin", "Heinrichs Weikamp", "OSTC 5"],
  ["garmin_descent_mk1-0001.bin", "Garmin", "Descent™ Mk1"],
]) {
  const dive = parse(vendor, product, readFileSync(join(dir, file)));
  if (!dive || dive.samples.length === 0) throw new Error(`failed to parse ${file}`);
}

if (parse("No Such", "Computer", Buffer.from([0])) !== null) {
  throw new Error("unknown device should not parse");
}

// Asyncify transport: a mock host echoes whatever was written, proving the
// custom iostream round-trips through async JS and back.
const writes = [];
globalThis.benthicHost = {
  async configure() {},
  async write(bytes) {
    writes.push(bytes);
    return bytes.length;
  },
  async read(size) {
    const source = writes.length ? writes[writes.length - 1] : new Uint8Array();
    return source.slice(0, size);
  },
  async poll() {
    return true;
  },
  available() {
    return 0;
  },
  async sleep() {},
  async close() {},
  event() {},
};

function callAsync(fn) {
  return new Promise((resolve) => {
    globalThis.benthicHost.result = resolve;
    fn();
  });
}

const selftest = JSON.parse(await callAsync(() => Module._benthic_dc_selftest(0)));
if (selftest.wrote !== 4 || selftest.text !== "ping") {
  throw new Error(`async transport selftest failed: ${JSON.stringify(selftest)}`);
}

const bleSelftest = JSON.parse(await callAsync(() => Module._benthic_dc_selftest(1)));
if (bleSelftest.wrote !== 4 || bleSelftest.text !== "ping") {
  throw new Error(`BLE iostream selftest failed: ${JSON.stringify(bleSelftest)}`);
}

console.log(
  `OK: ${descriptors.length} descriptors, ${shearwater.samples.length} samples, async transport ok`,
);
