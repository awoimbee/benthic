// Exercises public/divecomputer/api.mjs: the on-demand loader, the model list
// and the port request, with a fake `navigator.serial`.
globalThis.performance ??= { now: () => Date.now() };

const fakePort = {
  async open() {},
  async close() {},
  writable: { getWriter: () => ({ write: async (bytes) => bytes.length, releaseLock() {} }) },
  readable: {
    getReader: () => ({
      read: async () => ({ value: new Uint8Array([1, 2, 3]), done: false }),
      cancel: async () => {},
      releaseLock() {},
    }),
  },
};
Object.defineProperty(globalThis, "navigator", {
  value: { serial: { requestPort: async () => fakePort } },
  configurable: true,
});

const api = (await import("../../public/divecomputer/api.mjs")).default;
if (typeof api.serialSupported !== "function" || !api.serialSupported()) {
  throw new Error("supported() should be true with navigator.serial present");
}
const models = JSON.parse(await api.descriptorsJson());
if (models.length < 100) throw new Error(`only ${models.length} models`);
if (!models.some((m) => m.vendor === "Shearwater" && m.product === "Petrel 2")) {
  throw new Error("missing Shearwater Petrel 2");
}
if (!(await api.connect(0))) throw new Error("serial connect failed");
if (typeof api.bluetoothSupported !== "function") throw new Error("no bluetoothSupported");

console.log(`OK: api loader, ${models.length} models, port request`);
