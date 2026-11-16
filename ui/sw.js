const CACHE_NAME = 'somaris-shell-v1';
// precache-manifest for the versioned, same-origin static app shell.
const PRECACHE = [
  '/',
  '/manifest.webmanifest',
  '/ui/style.css',
  '/ui/components.css',
  '/ui/panel.css',
  '/ui/app.mjs',
  '/ui/components.mjs',
  '/ui/escape.mjs',
  '/ui/page-components.mjs',
  '/ui/batch-feedback.mjs',
  '/ui/snackbar.mjs',
  '/ui/snackbar-components.mjs',
  '/ui/api-docs.mjs',
  '/ui/favicon.svg',
  '/ui/favicon-32x32.png',
  '/ui/apple-touch-icon.png',
  '/ui/pwa-192.png',
  '/ui/pwa-512.png',
  '/ui/pwa-maskable-512.png',
];
const STATIC_PATHS = new Set(PRECACHE);

self.addEventListener('install', event => {
  event.waitUntil(caches.open(CACHE_NAME).then(cache => cache.addAll(PRECACHE)));
});

self.addEventListener('activate', event => {
  event.waitUntil(caches.keys().then(keys => Promise.all(
    keys.filter(key => key.startsWith('somaris-shell-') && key !== CACHE_NAME).map(key => caches.delete(key)),
  )).then(() => self.clients.claim()));
});

self.addEventListener('message', event => {
  if (event.data?.type === 'SKIP_WAITING') self.skipWaiting();
});

self.addEventListener('fetch', event => {
  const request = event.request;
  const url = new URL(request.url);
  if (request.method !== 'GET' || url.origin !== self.location.origin) return;
  if (url.pathname.startsWith('/api/')) return;
  const path = url.pathname === '/' ? '/' : url.pathname;
  if (!STATIC_PATHS.has(path)) return;

  event.respondWith(caches.open(CACHE_NAME).then(async cache => {
    const cached = await cache.match(request, {ignoreSearch:true});
    const update = fetch(request).then(response => {
      if (response.ok && !url.pathname.startsWith('/api/')) cache.put(request, response.clone());
      return response;
    });
    if (cached) {
      event.waitUntil(update.catch(() => undefined));
      return cached;
    }
    return update;
  }));
});
