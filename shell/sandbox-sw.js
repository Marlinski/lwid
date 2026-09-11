/**
 * Service Worker for a project's own sandbox origin (e.g.
 * <project-id>.lookwhatidid.xyz).
 *
 * This is sw.js's exact job, moved to its own origin: intercept every fetch
 * on this origin and serve decrypted files from an in-memory cache, so the
 * user's deployed app behaves like a normal static site. The difference is
 * *where* it runs — a dedicated origin per project means a malicious
 * project's own script (running same-origin with this SW, as
 * `sandbox="allow-scripts allow-same-origin"` always allows) can only ever
 * reach *this* project's storage/DOM, never the shell's (My Projects'
 * localStorage, write keys, etc.) or another project's.
 *
 * This whole origin is otherwise empty — sandbox.html (the one page that
 * ever loads directly here) registers this worker, then hands it files via
 * postMessage relayed from the shell. See sandbox.html for that handshake.
 *
 * Communication protocol (sandbox.html -> SW via postMessage):
 *
 *   { type: 'SET_FILES', files: Array<{ path, content, mimeType }>,
 *     viewer?: string | null, canEdit?: boolean }
 *     Replace the entire file cache. `content` is an ArrayBuffer.
 *
 *   { type: 'CLEAR_FILES' }
 *     Wipe the cache and forget the active viewer.
 *
 * Fetch interception: every request on this origin is served from cache
 * (there is nothing else here to fall through to).
 *
 * Viewers: same reserved prefixes as sw.js —
 *   /__viewer__/<path>  -> shell origin's /viewers/<active viewer>/<path>
 *   /__shared__/<path>  -> shell origin's /viewers/_shared/<path>
 *   /__lwid/files.json  -> { files, viewer, canEdit }
 * fetched cross-origin from the shell (its CORS is already wide open for
 * these — see build_cors() in lwid-server, default `cors_origins: ["*"]`).
 *
 * Re-hydration: identical need as sw.js (in-memory cache wiped when this SW
 * goes idle and gets terminated), but self.clients.matchAll() here can only
 * ever find clients on *this* origin — sandbox.html itself, which relays the
 * request up to the shell via window.parent.postMessage() and never
 * navigates away, so it's always around to ask.
 */

// ---------------------------------------------------------------------------
// In-memory file cache: path -> { content: Uint8Array, mimeType: string }
// ---------------------------------------------------------------------------
const fileCache = new Map();

let activeViewer = null;
let viewerCanEdit = false;

let hydratePromise = null;
let hydrateResolve = null;

function finishHydration() {
  if (hydrateResolve) hydrateResolve();
  hydratePromise = null;
  hydrateResolve = null;
}

// ---------------------------------------------------------------------------
// This origin's own shell origin — sandbox.html tells us explicitly (on
// SET_SHELL_ORIGIN at boot and again on every SET_FILES), rather than this
// guessing it from its own hostname: whether this origin is a real
// per-project subdomain or just this same origin (no sandbox_base_domain
// configured on the server) isn't something a hostname alone can tell apart.
//
// Like fileCache and activeViewer this is lost whenever the browser
// terminates an idle SW, and the default below is *wrong* on a real
// subdomain — so nothing that needs it may run before re-hydration has had
// its chance (see the top of handleRequest()).
// ---------------------------------------------------------------------------
let SHELL_ORIGIN = self.location.origin;

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
    if (event.data.shellOrigin) SHELL_ORIGIN = event.data.shellOrigin;
    if (event.ports && event.ports[0]) {
      event.ports[0].postMessage({ type: 'FILES_READY' });
    }
    finishHydration();
  } else if (type === 'CLEAR_FILES') {
    fileCache.clear();
    activeViewer = null;
    viewerCanEdit = false;
  } else if (type === 'SET_SHELL_ORIGIN') {
    if (event.data.origin) SHELL_ORIGIN = event.data.origin;
  }
});

// ---------------------------------------------------------------------------
// Fetch interception — everything on this origin only. A SW's fetch event
// fires for every request a page in its scope makes, cross-origin included
// (the injected lwid-sdk.js <script src>, a viewer's CDN libraries, the
// __viewer__/__shared__ proxy's own cross-origin fetch to the shell) — all
// of those must fall through to the real network untouched, not get looked
// up in *this* project's file cache and 404.
// ---------------------------------------------------------------------------
// The bridge page itself — must always come from the network/server, never
// this SW's per-project file cache. It can be navigated to again (e.g. a
// full page reload of the shell) after this SW is already installed and
// controlling its scope from an earlier session, and it isn't a project
// file, so a cache lookup for it would just 404.
const BRIDGE_PATH = '__lwid_sandbox__/sandbox.html';

self.addEventListener('fetch', (event) => {
  const url = new URL(event.request.url);
  if (url.origin !== self.location.origin) return;
  if (url.pathname.replace(/^\//, '') === BRIDGE_PATH) return;
  event.respondWith(handleRequest(url));
});

/**
 * Ask sandbox.html (the one persistent client on this origin) to relay a
 * re-hydration request up to the shell. Mirrors sw.js's requestHydration(),
 * just one hop further since the shell isn't a same-origin client anymore.
 */
function requestHydration() {
  if (hydratePromise) return hydratePromise;
  hydratePromise = new Promise((resolve) => { hydrateResolve = resolve; });
  self.clients
    .matchAll({ type: 'window', includeUncontrolled: true })
    .then((clients) => {
      if (clients.length === 0) { finishHydration(); return; }
      for (const c of clients) c.postMessage({ type: 'REQUEST_FILES' });
    })
    .catch(() => finishHydration());
  setTimeout(finishHydration, 5000);
  return hydratePromise;
}

function notFound() {
  return new Response('Not Found', {
    status: 404,
    headers: { 'Content-Type': 'text/plain' },
  });
}

// Fetched once per SW lifetime and inlined (not left as a <script src>)
// because in same-origin fallback mode (no sandbox_base_domain configured)
// SHELL_ORIGIN equals this SW's own origin — a plain <script src> to it
// would be a same-origin page request, which this same SW would intercept
// and 404 (nothing in the project's own file cache is named js/lwid-sdk.js).
// Inlining sidesteps that regardless of which mode is active.
let sdkSourceCache = null;
async function sdkScriptTag() {
  if (sdkSourceCache == null) {
    try {
      const res = await fetch(SHELL_ORIGIN + '/js/lwid-sdk.js', { cache: 'no-cache', mode: 'cors' });
      sdkSourceCache = res.ok ? await res.text() : '';
    } catch {
      sdkSourceCache = '';
    }
  }
  return sdkSourceCache
    ? `<script>${sdkSourceCache}</script>`
    : `<script src="${SHELL_ORIGIN}/js/lwid-sdk.js"></script>`; // last-resort fallback
}

/**
 * Inject the lwid SDK into an HTML document and return it as a Response.
 */
async function htmlResponse(bytes) {
  let html = new TextDecoder().decode(bytes);
  const scriptTag = await sdkScriptTag();

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
 * Fetch a viewer bundle asset from the shell origin (cross-origin — the
 * shell's CORS is wide open for static assets). HTML documents are run
 * through {@link htmlResponse} so the viewer gets `window.lwid`.
 */
async function serveShellAsset(path) {
  let res;
  try {
    res = await fetch(SHELL_ORIGIN + path, { cache: 'no-cache', mode: 'cors' });
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

/** Synthesize the project manifest a viewer reads to discover its files.
 * (Hydration, if needed, already happened at the top of handleRequest.) */
async function synthesizeFilesJson() {
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

// The narrow scope sandbox.html registers this SW under (see its own
// ENTRY_PREFIX comment) — only the content iframe's own top-level
// navigation ever carries this; everything that page itself then fetches
// (images, __viewer__/__shared__ assets, ...) is a plain root-relative path.
const ENTRY_PREFIX = '__lwid_sandbox__/entry/';

async function handleRequest(url) {
  let path = url.pathname.replace(/^\//, '');
  if (path.startsWith(ENTRY_PREFIX)) path = path.slice(ENTRY_PREFIX.length);
  try { path = decodeURIComponent(path); } catch { /* keep the raw form */ }

  // An empty cache means this SW instance has never been primed — either
  // brand new, or (far more often) just restarted after the browser
  // terminated it while idle, taking fileCache, activeViewer AND
  // SHELL_ORIGIN with it. Re-hydrate before handling *anything*: a viewer
  // asset or the SDK injection fetched with a forgotten SHELL_ORIGIN goes
  // to this sandbox origin instead of the shell, where it doesn't exist.
  // Normal loads never pay for this — files always arrive before the
  // content iframe is pointed anywhere.
  if (fileCache.size === 0) await requestHydration();

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

  if (path === '' || path.endsWith('/')) {
    path += 'index.html';
  }

  const entry = fileCache.get(path);

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
self.addEventListener('install', () => {
  self.skipWaiting();
});

self.addEventListener('activate', (event) => {
  event.waitUntil(self.clients.claim());
});
