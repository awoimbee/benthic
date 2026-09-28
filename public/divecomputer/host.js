/*
 * benthic — dive-computer transport host for the web shim.
 *
 * The wasm shim calls these functions from its dc_custom_open iostream
 * callbacks (via Asyncify). This module implements them over WebSerial.
 *
 * Usage:
 *   import { installSerialHost, webSerialSupported } from './host.js';
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
