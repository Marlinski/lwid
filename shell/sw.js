/**
 * Service Worker for "lookwhatidid"
 *
 * Purpose: intercept fetch requests from the sandboxed iframe and serve
 * decrypted files from an in-memory cache, so the user's deployed app
 * behaves like a normal static site.
 *
 * Communication protocol (shell SPA -> SW via postMessage):
 *
 *   { type: 'SET_FILES', files: Array<{ path, content, mimeType }>,
 *     viewer?: string | null, canEdit?: boolean }
 *     Replace the entire file cache. `content` is an ArrayBuffer (structured
 *     clone converts Uint8Array to ArrayBuffer during transfer). `viewer`, when
 *     set, names a front-end shim (see below); `canEdit` says whether the
 *     current link carries a write key.
 *
 *   { type: 'CLEAR_FILES' }
 *     Wipe the cache and forget the active viewer.
 *
 * Fetch interception:
 *   Requests whose URL path starts with /sandbox/ are served from cache.
 *   Everything else falls through to the network untouched.
 *
 * Viewers:
 *   When a `viewer` is active, the sandbox root (`/sandbox/`) renders that
 *   viewer's own `index.html` (pulled from `/viewers/<id>/` on the shell
 *   origin) instead of the project files. Two reserved prefixes expose the
 *   shell-origin bundles to the running viewer:
 *
 *     /sandbox/__viewer__/<path>  -> /viewers/<active viewer>/<path>
 *     /sandbox/__shared__/<path>  -> /viewers/_shared/<path>
 *
 *   and one synthesized endpoint lets the viewer discover the project:
 *
 *     /sandbox/__lwid/files.json  -> { files: [{ path, size, mimeType }],
 *                                      viewer, canEdit }
 *
 *   The project's own files stay reachable at their real paths, so a viewer
 *   fetches e.g. `/sandbox/analysis.ipynb` or `/sandbox/data/foo.csv` directly.
 *
 * Re-hydration:
 *   The file cache is in-memory, so it is wiped whenever the browser
 *   terminates this (idle) Service Worker. On a subsequent /sandbox/ request
 *   the cache is empty and every navigation would 404 until the shell page is
 *   reloaded. To avoid that, an empty-cache miss asks a live shell client to
 *   re-send its (already-decrypted, in-memory) files via SET_FILES, then the
 *   request is retried — so intra-app navigation keeps working across SW
 *   restarts without ever persisting plaintext to disk. The SET_FILES message
 *   also re-declares the active viewer.
 */

// ---------------------------------------------------------------------------
// In-memory file cache: path -> { content: Uint8Array, mimeType: string }
// ---------------------------------------------------------------------------
const fileCache = new Map();

// Active viewer shim. When non-null, the sandbox root serves the viewer's own
// index.html and the reserved __viewer__ / __shared__ prefixes are live.
let activeViewer = null;
let viewerCanEdit = false;

// Pending re-hydration request (shared so concurrent misses coalesce).
let hydratePromise = null;
let hydrateResolve = null;

function finishHydration() {
  if (hydrateResolve) hydrateResolve();
  hydratePromise = null;
  hydrateResolve = null;
}

// ---------------------------------------------------------------------------
// MIME type helper
// ---------------------------------------------------------------------------
function guessMimeType(path) {
  const ext = (path.match(/\.[^.]+$/) || [''])[0].toLowerCase();
  switch (ext) {
    case '.html':   return 'text/html';
    case '.css':    return 'text/css';
    case '.js':
    case '.mjs':    return 'application/javascript';
    case '.json':   return 'application/json';
    case '.ipynb':  return 'application/json';
    case '.map':    return 'application/json';
    case '.csv':    return 'text/csv';
    case '.md':
    case '.markdown': return 'text/markdown';
    case '.txt':    return 'text/plain';
    case '.py':     return 'text/x-python';
    case '.toml':
    case '.yaml':
    case '.yml':    return 'text/plain';
    case '.xml':    return 'application/xml';
    case '.svg':    return 'image/svg+xml';
    case '.png':    return 'image/png';
    case '.jpg':
    case '.jpeg':   return 'image/jpeg';
    case '.gif':    return 'image/gif';
    case '.webp':   return 'image/webp';
    case '.ico':    return 'image/x-icon';
    case '.woff':   return 'font/woff';
    case '.woff2':  return 'font/woff2';
    case '.wasm':   return 'application/wasm';
    case '.sqlite':
    case '.db':     return 'application/x-sqlite3';
    default:        return 'application/octet-stream';
  }
}

// ---------------------------------------------------------------------------
// Message handler
// ---------------------------------------------------------------------------
self.addEventListener('message', (event) => {
  const { type, files } = event.data || {};

  if (type === 'SET_FILES') {
    fileCache.clear();
    for (const file of files) {
      // content arrives as ArrayBuffer from structured clone; wrap it
      const bytes = file.content instanceof ArrayBuffer
        ? new Uint8Array(file.content)
        : file.content;
      fileCache.set(file.path, {
        content: bytes,
        mimeType: file.mimeType || guessMimeType(file.path),
      });
    }
    activeViewer = event.data.viewer || null;
    viewerCanEdit = !!event.data.canEdit;
    // Acknowledge that files are cached so the page can load the iframe.
    if (event.ports && event.ports[0]) {
      event.ports[0].postMessage({ type: 'FILES_READY' });
    }
    // Unblock any /sandbox/ request that was waiting on re-hydration.
    finishHydration();
  } else if (type === 'CLEAR_FILES') {
    fileCache.clear();
    activeViewer = null;
    viewerCanEdit = false;
  }
});

// ---------------------------------------------------------------------------
// Fetch interception
// ---------------------------------------------------------------------------
self.addEventListener('fetch', (event) => {
  const url = new URL(event.request.url);

  // Only intercept paths under /sandbox/
  if (!url.pathname.startsWith('/sandbox/')) {
    return; // fall through to network
  }

  event.respondWith(handleSandboxRequest(url));
});

/**
 * Ask a live shell client to re-send its decrypted files (SET_FILES). Called
 * when the cache is empty because the SW was terminated and restarted. Resolves
 * once files arrive (via finishHydration) or after a short timeout. Concurrent
 * callers share the same in-flight request.
 */
function requestHydration() {
  if (hydratePromise) return hydratePromise;
  hydratePromise = new Promise((resolve) => { hydrateResolve = resolve; });
  self.clients
    .matchAll({ type: 'window', includeUncontrolled: true })
    .then((clients) => {
      // The shell (holder of the decrypted files) lives outside /sandbox/;
      // the sandboxed app iframe cannot re-hydrate, so skip it.
      const shells = clients.filter(
        (c) => !new URL(c.url).pathname.startsWith('/sandbox/'),
      );
      if (shells.length === 0) {
        finishHydration(); // nobody to ask — fail fast to a 404
        return;
      }
      for (const c of shells) c.postMessage({ type: 'REQUEST_FILES' });
    })
    .catch(() => finishHydration());
  // Safety net: never hang a request forever.
  setTimeout(finishHydration, 5000);
  return hydratePromise;
}

function notFound() {
  return new Response('Not Found', {
    status: 404,
    headers: { 'Content-Type': 'text/plain' },
  });
}

/**
 * Inject the lwid SDK into an HTML document and return it as a Response.
 * Shared by cached project pages and viewer bundles alike.
 */
function htmlResponse(bytes) {
  let html = new TextDecoder().decode(bytes);
  const scriptTag = '<script src="/js/lwid-sdk.js"></script>';

  const headMatch = html.match(/<head(\s[^>]*)?>/i);
  if (headMatch) {
    const insertPos = headMatch.index + headMatch[0].length;
    html = html.slice(0, insertPos) + scriptTag + html.slice(insertPos);
  } else {
    const htmlMatch = html.match(/<html(\s[^>]*)?>/i);
    if (htmlMatch) {
      const insertPos = htmlMatch.index + htmlMatch[0].length;
      html = html.slice(0, insertPos) + '<head>' + scriptTag + '</head>' + html.slice(insertPos);
    } else {
      html = scriptTag + '\n' + html;
    }
  }

  return new Response(html, {
    status: 200,
    headers: { 'Content-Type': 'text/html' },
  });
}

/**
 * Serve a static asset from the shell origin (a viewer bundle). HTML documents
 * are run through {@link htmlResponse} so the viewer gets `window.lwid`.
 */
async function serveShellAsset(path) {
  let res;
  try {
    res = await fetch(path, { cache: 'no-cache' });
  } catch {
    return notFound();
  }
  if (!res.ok) return notFound();

  const buf = await res.arrayBuffer();
  const ct = res.headers.get('Content-Type') || guessMimeType(path);
  if (ct.startsWith('text/html')) {
    return htmlResponse(new Uint8Array(buf));
  }
  return new Response(buf, { status: 200, headers: { 'Content-Type': ct } });
}

/** Synthesize the project manifest a viewer reads to discover its files. */
async function synthesizeFilesJson() {
  if (fileCache.size === 0) await requestHydration();

  const files = [];
  for (const [path, entry] of fileCache) {
    files.push({ path, size: entry.content.byteLength, mimeType: entry.mimeType });
  }
  files.sort((a, b) => (a.path < b.path ? -1 : a.path > b.path ? 1 : 0));

  return new Response(
    JSON.stringify({ files, viewer: activeViewer, canEdit: viewerCanEdit }),
    { status: 200, headers: { 'Content-Type': 'application/json', 'Cache-Control': 'no-store' } },
  );
}

async function handleSandboxRequest(url) {
  // Strip the /sandbox/ prefix and percent-decode (cache keys are raw paths).
  let path = url.pathname.slice('/sandbox/'.length);
  try { path = decodeURIComponent(path); } catch { /* keep the raw form */ }

  // ── Reserved viewer namespaces ───────────────────────────────────────────
  if (path === '__lwid/files.json') {
    return synthesizeFilesJson();
  }
  if (path === '__viewer__' || path.startsWith('__viewer__/')) {
    if (!activeViewer) return notFound();
    const rest = path.slice('__viewer__'.length).replace(/^\//, '') || 'index.html';
    return serveShellAsset(`/viewers/${activeViewer}/${rest}`);
  }
  if (path === '__shared__' || path.startsWith('__shared__/')) {
    const rest = path.slice('__shared__'.length).replace(/^\//, '');
    return serveShellAsset(`/viewers/_shared/${rest}`);
  }

  // Treat empty path or trailing slash as a directory -> index.html
  if (path === '' || path.endsWith('/')) {
    path += 'index.html';
  }

  let entry = fileCache.get(path);

  // Empty cache ⇒ the SW was almost certainly restarted after going idle.
  // Ask the shell to re-send the files (and re-declare the viewer), then
  // retry once. (A non-empty cache that simply lacks this path is a genuine
  // 404 — don't re-hydrate.)
  if (!entry && fileCache.size === 0) {
    await requestHydration();
    entry = fileCache.get(path);
  }

  // With a viewer active, the sandbox root renders the viewer shell rather
  // than a project file. (detectViewer() only picks a viewer when the project
  // has no HTML of its own, so this never shadows a real index.html.)
  if (!entry && activeViewer && path === 'index.html') {
    return serveShellAsset(`/viewers/${activeViewer}/index.html`);
  }

  if (entry) {
    if (entry.mimeType === 'text/html') {
      return htmlResponse(entry.content);
    }
    return new Response(entry.content, {
      status: 200,
      headers: { 'Content-Type': entry.mimeType },
    });
  }

  return notFound();
}

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

// Activate immediately without waiting for old SW to retire
self.addEventListener('install', () => {
  self.skipWaiting();
});

// Take control of all open clients right away
self.addEventListener('activate', (event) => {
  event.waitUntil(self.clients.claim());
});
