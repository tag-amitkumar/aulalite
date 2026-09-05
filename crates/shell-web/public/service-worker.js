// AulaLite PWA service worker — offline shell plus bounded static caching.
// Tenant-scoped/API responses are deliberately never cached.
/* eslint-disable no-undef */

// Production registers this worker with a deployment fingerprint. Cache names
// therefore rotate when either the app shell or runtime configuration changes.
const DEPLOY_VERSION =
  new URL(self.location.href).searchParams.get("v") || "development";
const SAFE_VERSION = DEPLOY_VERSION.replace(/[^a-zA-Z0-9._-]/g, "-");
const CACHE_VERSION = `aulalite-v2-${SAFE_VERSION}`;
const SHELL_CACHE = `${CACHE_VERSION}-shell`;
const RUNTIME_CACHE = `${CACHE_VERSION}-runtime`;
const MAX_RUNTIME_ENTRIES = 96;

const PRECACHE_URLS = [
  "/",
  "/manifest.webmanifest",
  "/assets/tokens.css",
  "/assets/components.css",
  "/assets/app-bootstrap.js",
  "/assets/feature-loader.js",
  "/assets/marketing-motion.js",
  "/assets/brand/aulalite-mark.svg",
  "/assets/brand/aulalite-wordmark.svg",
  "/assets/brand/app-icon-32.png",
  "/assets/brand/app-icon-256.png",
  "/assets/brand/app-icon-512.png",
  "/assets/fonts/Inter-Variable.woff2",
  "/assets/fonts/outfit-latin-wght-normal.woff2",
  "/assets/fonts/SourceSerif4-Variable.woff2",
];

const NEVER_CACHE_PATHS = new Set([
  "/runtime-config.js",
  "/service-worker.js",
  "/firebase-messaging-sw.js",
]);

self.addEventListener("install", (event) => {
  event.waitUntil(
    caches.open(SHELL_CACHE).then((cache) =>
      // One optional asset must not prevent a new deployment from activating.
      Promise.allSettled(
        PRECACHE_URLS.map((url) =>
          cache.add(new Request(url, { cache: "reload" }))
        )
      )
    )
  );
  self.skipWaiting();
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    (async () => {
      const keys = await caches.keys();
      await Promise.all(
        keys
          .filter(
            (key) =>
              key.startsWith("aulalite-") &&
              key !== SHELL_CACHE &&
              key !== RUNTIME_CACHE
          )
          .map((key) => caches.delete(key))
      );
      await self.clients.claim();
    })()
  );
});

function isCacheableAsset(request, url) {
  if (url.origin !== self.location.origin) return false;
  if (url.pathname.startsWith("/v1/")) return false;
  if (NEVER_CACHE_PATHS.has(url.pathname)) return false;
  if (request.headers.has("authorization")) return false;

  // Signed download URLs are credentials. Even when a storage proxy happens
  // to share this origin, do not persist them in a browser-wide cache.
  const queryKeys = Array.from(url.searchParams.keys()).map((key) =>
    key.toLowerCase()
  );
  if (
    queryKeys.some(
      (key) =>
        key.includes("signature") ||
        key.includes("credential") ||
        key === "token"
    )
  ) {
    return false;
  }

  // Cache only executable/presentation assets. Never cache arbitrary GETs such
  // as exports, tenant files or media streams.
  return (
    ["script", "style", "font", "image", "worker"].includes(
      request.destination
    ) ||
    url.pathname.startsWith("/vendor/") ||
    url.pathname.endsWith(".wasm")
  );
}

function responseMayBeCached(response) {
  if (!response || !response.ok || response.type !== "basic") return false;
  const cacheControl = (response.headers.get("cache-control") || "").toLowerCase();
  return !cacheControl.includes("no-store") && !cacheControl.includes("private");
}

async function trimRuntimeCache(cache) {
  const keys = await cache.keys();
  const overflow = keys.length - MAX_RUNTIME_ENTRIES;
  if (overflow > 0) {
    await Promise.all(keys.slice(0, overflow).map((key) => cache.delete(key)));
  }
}

async function refreshRuntimeAsset(cache, request) {
  // Bypass the browser's HTTP cache so a background refresh really checks the
  // current deployment instead of re-caching a stale stable-path response.
  const response = await fetch(new Request(request, { cache: "no-cache" }));
  if (responseMayBeCached(response)) {
    await cache.put(request, response.clone());
    await trimRuntimeCache(cache);
  }
  return response;
}

async function staticAssetResponse(event, request) {
  const cache = await caches.open(RUNTIME_CACHE);
  const cached = await caches.match(request);
  const refresh = refreshRuntimeAsset(cache, request);

  if (cached) {
    // Stale-while-revalidate gives repeat visits instant assets without pinning
    // stable URLs forever. Network failure leaves the known-good copy intact.
    event.waitUntil(refresh.catch(() => undefined));
    return cached;
  }
  return refresh;
}

self.addEventListener("fetch", (event) => {
  const request = event.request;
  if (request.method !== "GET") return;

  const url = new URL(request.url);
  if (url.origin !== self.location.origin) return;
  if (
    url.pathname.startsWith("/v1/") ||
    NEVER_CACHE_PATHS.has(url.pathname) ||
    request.headers.has("authorization")
  ) {
    return;
  }

  if (request.mode === "navigate") {
    event.respondWith(
      fetch(new Request(request, { cache: "no-cache" }))
        .then((response) => {
          const contentType = response.headers.get("content-type") || "";
          if (response.ok && contentType.includes("text/html")) {
            event.waitUntil(
              caches
                .open(SHELL_CACHE)
                .then((cache) => cache.put("/", response.clone()))
            );
          }
          return response;
        })
        .catch(async () => {
          const shell = await caches.match("/", { cacheName: SHELL_CACHE });
          return (
            shell ||
            new Response(
              `<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width,initial-scale=1,viewport-fit=cover">
  <meta name="theme-color" content="#10251f">
  <title>AulaLite is offline</title>
  <style>
    :root { color-scheme: light dark; font-family: Inter, system-ui, sans-serif; }
    body { min-height: 100dvh; margin: 0; display: grid; place-items: center; padding: 1.5rem; box-sizing: border-box; color: #171a17; background: #f6f0e6; }
    main { width: min(100%, 32rem); padding: clamp(1.5rem, 6vw, 2.5rem); box-sizing: border-box; border: 1px solid rgba(18,22,20,.16); border-radius: 1.25rem; background: #fffdf7; box-shadow: 0 1.5rem 4rem rgba(18,22,20,.12); }
    p { line-height: 1.65; color: #6d6558; }
    a { display: inline-flex; min-height: 2.75rem; align-items: center; margin-top: .5rem; padding: 0 1rem; border-radius: .65rem; color: white; background: #244f43; font-weight: 750; text-decoration: none; }
    a:focus-visible { outline: 3px solid #765019; outline-offset: 3px; }
    @media (prefers-color-scheme: dark) { body { color: #f1ead9; background: #0e1512; } main { border-color: rgba(241,234,217,.16); background: #17221d; } p { color: #ada48f; } a { background: #386658; } }
  </style>
</head>
<body>
  <main>
    <p aria-hidden="true">AulaLite</p>
    <h1>You’re offline</h1>
    <p>Reconnect to refresh your academy workspace. Any course content already open in this session remains available where supported.</p>
    <a href="/">Try again</a>
  </main>
</body>
</html>`,
              { headers: { "Content-Type": "text/html; charset=utf-8" } }
            )
          );
        })
    );
    return;
  }

  if (isCacheableAsset(request, url)) {
    event.respondWith(staticAssetResponse(event, request));
  }
});
