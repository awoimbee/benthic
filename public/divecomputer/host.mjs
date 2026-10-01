/*
 * benthic — dive-computer transport host for the web shim.
 *
 * The wasm shim calls these functions from its dc_custom_open iostream
 * callbacks (via Asyncify). This module implements them over WebSerial.
 *
 * Usage:
 *   import { installSerialHost, webSerialSupported } from './host.mjs';
 *   installSerialHost();
 *   await window.benthicDc.requestPort(); // from a user gesture
 */

function concat(left, right) {
  const out = new Uint8Array(left.length + right.length);
  out.set(left, 0);
  out.set(right, left.length);
  return out;
}

/** A WebSerial-backed host. */
export function createSerialHost() {
  let port = null;
  let reader = null;
  let writer = null;
  let buffered = new Uint8Array(0);
  let pendingRead = null;

  async function readWithTimeout(ms) {
    if (!pendingRead) {
      pendingRead = reader.read().finally(() => {
        pendingRead = null;
      });
    }
    if (ms <= 0) {
      const result = await pendingRead;
      if (result.done) throw new Error("serial port closed");
      if (result.value) buffered = concat(buffered, result.value);
      return true;
    }
    const timeout = new Promise((resolve) => setTimeout(() => resolve("timeout"), ms));
    const result = await Promise.race([pendingRead, timeout]);
    if (result === "timeout") return false;
    if (result.done) throw new Error("serial port closed");
    if (result.value) buffered = concat(buffered, result.value);
    return true;
  }

  return {
    async requestPort() {
      port = await navigator.serial.requestPort();
      return true;
    },

    hasPort() {
      return port !== null;
    },

    async configure(baudRate) {
      if (!port) throw new Error("no serial port selected");
      await port.open({ baudRate });
      writer = port.writable.getWriter();
      reader = port.readable.getReader();
      buffered = new Uint8Array(0);
    },

    async read(size, timeout) {
      const deadline = performance.now() + (timeout > 0 ? timeout : 2000);
      while (buffered.length < size) {
        const remaining = deadline - performance.now();
        if (remaining <= 0 && buffered.length > 0) break;
        const got = await readWithTimeout(remaining > 0 ? remaining : 0);
        if (!got && buffered.length > 0) break;
        if (!got) break;
      }
      const n = Math.min(size, buffered.length);
      const out = buffered.slice(0, n);
      buffered = buffered.slice(n);
      return out;
    },

    async write(bytes) {
      await writer.write(bytes);
      return bytes.length;
    },

    async poll(timeout) {
      if (buffered.length > 0) return true;
      return readWithTimeout(timeout);
    },

    available() {
      return buffered.length;
    },

    async sleep(ms) {
      await new Promise((resolve) => setTimeout(resolve, ms));
    },

    async close() {
      try {
        if (reader) {
          await reader.cancel();
          reader.releaseLock();
        }
      } catch (error) {
        /* already released */
      }
      try {
        if (writer) writer.releaseLock();
      } catch (error) {
        /* already released */
      }
      reader = null;
      writer = null;
      buffered = new Uint8Array(0);
      if (port) {
        await port.close();
        port = null;
      }
    },

    event() {},
  };
}

export function webSerialSupported() {
  return typeof navigator !== "undefined" && "serial" in navigator;
}

/** Installs a host as globalThis.benthicHost (the shim looks there). */
export function install(host = createSerialHost()) {
  globalThis.benthicHost = host;
  return host;
}

export function installSerialHost() {
  return install(createSerialHost());
}

/*
 * Known BLE GATT serial services, in preference order. Mirrors Subsurface's
 * qt-ble.cpp table: BLE never standardised serial, so every vendor invented
 * its own. A device with a notify characteristic and a write characteristic
 * inside one of these services is treated as a serial stream.
 */
export const SERIAL_SERVICE_UUIDS = [
  "0000fefb-0000-1000-8000-00805f9b34fb", // Heinrichs-Weikamp (Telit/Stollmann)
  "2456e1b9-26e2-8f83-e744-f34f01e9d701", // Heinrichs-Weikamp (U-Blox)
  "544e326b-5b72-c6b0-1c46-41c1bc448118", // Mares BlueLink Pro
  "98ae7120-e62e-11e3-badd-0002a5d5c51b", // Suunto (EON Steel/Core, G5)
  "cb3c4555-d670-4670-bc20-b61dbc851e9a", // Pelagic (i770R, i200C, Pro Plus X, Geo 4.0)
  "ca7b0001-f785-4c38-b599-c7c5fbadb034", // Pelagic (i330R, DSX)
  "fdcdeaaa-295d-470e-bf15-04217b7aa0a0", // ScubaPro (G2, G3)
  "fe25c237-0ece-443c-b0aa-e02033e7029d", // Shearwater (Perdix/Teric/Peregrine/Tern)
  "1aa44039-1667-4b29-87cc-dfecaaf31d97", // Shearwater (Perdix 3)
  "0000fcef-0000-1000-8000-00805f9b34fb", // Divesoft
  "6e400001-b5a3-f393-e0a9-e50e24dc10b8", // Cressi
  "6e400001-b5a3-f393-e0a9-e50e24dcca9e", // Nordic Semi UART
  "00000001-8c3b-4f2c-a59e-8c08224f3253", // Halcyon Symbios
  "84968ffe-d26d-478a-b953-5010bcf58bca", // Seac
];

// The BLE ioctl operations the shim forwards (see web/divecomputer/shim.c).
const BLE_OP_NAME = 0;
const BLE_OP_PINCODE = 1;
const BLE_OP_GET_ACCESSCODE = 2;
const BLE_OP_SET_ACCESSCODE = 3;
const BLE_OP_CHARACTERISTIC_READ = 4;

function uuidFromBytes(bytes) {
  const hex = [...bytes].map((b) => b.toString(16).padStart(2, "0")).join("");
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}

/** A WebBluetooth-backed host for BLE dive computers. */
export function createBleHost() {
  let device = null;
  let server = null;
  let service = null;
  let writeCharacteristic = null;
  let notifyCharacteristic = null;
  let readCharacteristic = null;
  let accessCode = new Uint8Array(0);
  let pinCode = "";
  let packets = [];
  let waiters = [];

  function enqueue(value) {
    const data = value instanceof DataView ? new Uint8Array(value.buffer.slice(0)) : new Uint8Array(value);
    packets.push(data);
    const resume = waiters;
    waiters = [];
    for (const resolve of resume) resolve("packet");
  }

  function waitForPacket(ms) {
    return new Promise((resolve) => {
      const timer = setTimeout(() => resolve("timeout"), ms);
      waiters.push(() => {
        clearTimeout(timer);
        resolve("packet");
      });
    });
  }

  async function pickService() {
    const services = await server.getPrimaryServices();
    for (const uuid of SERIAL_SERVICE_UUIDS) {
      const candidate = services.find((s) => s.uuid === uuid);
      if (!candidate) continue;
      const characteristics = await candidate.getCharacteristics();
      const write = characteristics.find(
        (c) => c.properties.write || c.properties.writeWithoutResponse,
      );
      const notify = characteristics.find((c) => c.properties.notify || c.properties.indicate);
      const read = characteristics.find((c) => c.properties.read);
      if (write && (notify || read)) {
        service = candidate;
        writeCharacteristic = write;
        notifyCharacteristic = notify;
        readCharacteristic = read;
        return;
      }
    }
    throw new Error("no known serial service on this device");
  }

  return {
    async requestDevice() {
      device = await navigator.bluetooth.requestDevice({
        acceptAllDevices: true,
        optionalServices: SERIAL_SERVICE_UUIDS,
      });
      return true;
    },

    hasDevice() {
      return device !== null;
    },

    async configure() {
      if (!device) throw new Error("no Bluetooth device selected");
      server = await device.gatt.connect();
      await pickService();
      packets = [];
      waiters = [];
      if (notifyCharacteristic) {
        notifyCharacteristic.addEventListener("characteristicvaluechanged", (event) => {
          enqueue(event.target.value);
        });
        await notifyCharacteristic.startNotifications();
      }
    },

    async read(size, timeout) {
      const deadline = performance.now() + (timeout > 0 ? timeout : 2000);
      while (packets.length === 0 && notifyCharacteristic) {
        const remaining = deadline - performance.now();
        if (remaining <= 0) break;
        await waitForPacket(remaining);
      }
      if (packets.length === 0) {
        if (!readCharacteristic) return new Uint8Array(0);
        const value = await readCharacteristic.readValue();
        return new Uint8Array(value.buffer).slice(0, size);
      }
      return packets.shift().slice(0, size);
    },

    async write(bytes) {
      await writeCharacteristic.writeValue(bytes);
      return bytes.length;
    },

    async poll(timeout) {
      if (packets.length > 0) return true;
      if (!notifyCharacteristic) return false;
      return (await waitForPacket(timeout > 0 ? timeout : 1)) === "packet";
    },

    available() {
      return packets.length;
    },

    async sleep(ms) {
      await new Promise((resolve) => setTimeout(resolve, ms));
    },

    async close() {
      try {
        if (notifyCharacteristic) await notifyCharacteristic.stopNotifications();
      } catch (error) {
        /* ignore */
      }
      try {
        if (server) server.disconnect();
      } catch (error) {
        /* ignore */
      }
      server = null;
      service = null;
      writeCharacteristic = null;
      notifyCharacteristic = null;
      readCharacteristic = null;
      packets = [];
      waiters = [];
      device = null;
    },

    async bleIoctl(op, bytes) {
      switch (op) {
        case BLE_OP_NAME: {
          const name = (device && device.name) || "";
          return new TextEncoder().encode(`${name}\0`);
        }
        case BLE_OP_PINCODE:
          return new TextEncoder().encode(`${pinCode}\0`);
        case BLE_OP_GET_ACCESSCODE:
          return accessCode;
        case BLE_OP_SET_ACCESSCODE:
          accessCode = new Uint8Array(bytes);
          return undefined;
        case BLE_OP_CHARACTERISTIC_READ: {
          const uuid = uuidFromBytes(bytes.slice(0, 16));
          const characteristic = await service.getCharacteristic(uuid);
          const value = await characteristic.readValue();
          return new Uint8Array(value.buffer);
        }
        default:
          throw new Error(`unsupported BLE ioctl ${op}`);
      }
    },

    event() {},
  };
}

export function webBluetoothSupported() {
  return typeof navigator !== "undefined" && "bluetooth" in navigator;
}
