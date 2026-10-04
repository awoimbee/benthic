// benthic service worker.
//
// A local-first dive log has to open on a boat with no signal, so the app
// shell and every same-origin asset are cached at runtime. The first visit
// populates the cache; later visits are served from it when the network is
// unavailable. HTML navigations fall back to the cached shell.

const CACHE = "benthic-v1";
const SHELL = "./";

self.addEventListener("install", (event) => {
  self.skipWaiting();
  event.waitUntil(
    caches
      .open(CACHE)
      .then((cache) => cache.add(SHELL))
      .catch(() => undefined),
  );
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    (async () => {
      const keys = await caches.keys();
      await Promise.all(keys.filter((key) => key !== CACHE).map((key) => caches.delete(key)));
      await self.clients.claim();
    })(),
  );
});

self.addEventListener("fetch", (event) => {
  const request = event.request;
  if (request.method !== "GET") return;

  const url = new URL(request.url);
  if (url.origin !== self.location.origin) return;
  // Never interfere with range requests used by the streaming downloaders.
  if (request.headers.has("range")) return;

  event.respondWith(
    (async () => {
      const cache = await caches.open(CACHE);
      try {
        const response = await fetch(request);
        if (response && response.status === 200 && response.type === "basic") {
          cache.put(request, response.clone());
        }
        return response;
      } catch (error) {
        const cached = await cache.match(request, { ignoreSearch: true });
        if (cached) return cached;
        if (request.mode === "navigate") {
          const shell = await cache.match(SHELL);
          if (shell) return shell;
        }
        throw error;
      }
    })(),
  );
});
