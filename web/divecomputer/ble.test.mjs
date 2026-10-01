// Exercises public/divecomputer/host.mjs BLE support against a fake
// navigator.bluetooth GATT server.
globalThis.performance ??= { now: () => Date.now() };

function makeCharacteristic(uuid, properties) {
  const listeners = [];
  return {
    uuid,
    properties,
    written: [],
    async writeValue(bytes) {
      this.written.push(new Uint8Array(bytes));
    },
    async readValue() {
      return new DataView(new Uint8Array([1, 2, 3, 4]).buffer);
    },
    async startNotifications() {
      this.notifying = true;
    },
    async stopNotifications() {
      this.notifying = false;
    },
    addEventListener(type, callback) {
      if (type === "characteristicvaluechanged") listeners.push(callback);
    },
    emit(bytes) {
      for (const callback of listeners) {
        callback({ target: { value: new DataView(bytes.buffer) } });
      }
    },
  };
}

const writeChar = makeCharacteristic("fe25c237-0ece-443c-b0aa-e02033e7029d", {
  write: true,
  writeWithoutResponse: false,
});
const notifyChar = makeCharacteristic("fe25c237-0ece-443c-b0aa-e02033e7029e", {
  notify: true,
});
const service = {
  uuid: "fe25c237-0ece-443c-b0aa-e02033e7029d",
  async getCharacteristics() {
    return [writeChar, notifyChar];
  },
  async getCharacteristic(uuid) {
    return [writeChar, notifyChar].find((c) => c.uuid === uuid);
  },
};
const server = {
  async getPrimaryServices() {
    return [service];
  },
  disconnect() {},
};
const device = { name: "Petrel 2", gatt: { connect: async () => server } };

Object.defineProperty(globalThis, "navigator", {
  value: { bluetooth: { requestDevice: async () => device } },
  configurable: true,
});

const { createBleHost, webBluetoothSupported, SERIAL_SERVICE_UUIDS } = await import("../../public/divecomputer/host.mjs");
if (!webBluetoothSupported()) throw new Error("bluetooth should be supported");
if (SERIAL_SERVICE_UUIDS.length < 10) throw new Error("service table too small");

const host = createBleHost();
await host.requestDevice();
await host.configure();
await host.write(new Uint8Array([1, 2, 3]));
if (writeChar.written.length !== 1 || writeChar.written[0][2] !== 3) {
  throw new Error("write did not reach the characteristic");
}

notifyChar.emit(new Uint8Array([9, 8, 7]));
const chunk = await host.read(3, 1000);
if (chunk.length !== 3 || chunk[0] !== 9) throw new Error("read did not return the notification");

if (await host.poll(10) !== false) throw new Error("poll should time out on an empty queue");

const name = await host.bleIoctl(0, new Uint8Array(16));
if (!new TextDecoder().decode(name).startsWith("Petrel 2")) throw new Error("bad device name");
const uuidBytes = new Uint8Array([
  0xfe, 0x25, 0xc2, 0x37, 0x0e, 0xce, 0x44, 0x3c, 0xb0, 0xaa, 0xe0, 0x20, 0x33, 0xe7, 0x02, 0x9d,
]);
const characteristic = await host.bleIoctl(4, uuidBytes);
if (characteristic[0] !== 1) throw new Error("characteristic read failed");

await host.close();
console.log("OK: BLE host (connect, write, notify, read, ioctl)");
