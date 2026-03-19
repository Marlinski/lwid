/**
 * Service Worker for "lookwhatidid"
 *
 * Purpose: intercept fetch requests from the sandboxed iframe and serve
 * decrypted files from an in-memory cache, so the user's deployed app
 * behaves like a normal static site.
 *
 * Communication protocol (shell SPA -> SW via postMessage):
 *
 *   { type: 'SET_FILES', files: Array<{ path, content, mimeType }> }
 *     Replace the entire file cache. `content` is an ArrayBuffer (structured
 *     clone converts Uint8Array to ArrayBuffer during transfer).
 *
 *   { type: 'CLEAR_FILES' }
 *     Wipe the cache.
 *
 * Fetch interception:
 *   Requests whose URL path starts with /sandbox/ are served from cache.
 *   Everything else falls through to the network untouched.
 */

// ---------------------------------------------------------------------------
// In-memory file cache: path -> { content: Uint8Array, mimeType: string }
// ---------------------------------------------------------------------------
const fileCache = new Map();

// ---------------------------------------------------------------------------
// MIME type helper
// ---------------------------------------------------------------------------
function guessMimeType(path) {
  const ext = (path.match(/\.[^.]+$/) || [''])[0].toLowerCase();
  switch (ext) {
    case '.html':   return 'text/html';
    case '.css':    return 'text/css';
    case '.js':     return 'application/javascript';
    case '.json':   return 'application/json';
    case '.csv':    return 'text/csv';
    case '.svg':    return 'image/svg+xml';
    case '.png':    return 'image/png';
    case '.jpg':
    case '.jpeg':   return 'image/jpeg';
    case '.gif':    return 'image/gif';
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
    // Acknowledge that files are cached so the page can load the iframe.
    if (event.ports && event.ports[0]) {
      event.ports[0].postMessage({ type: 'FILES_READY' });
    }
  } else if (type === 'CLEAR_FILES') {
    fileCache.clear();
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

function handleSandboxRequest(url) {
  // Strip the /sandbox/ prefix to derive the file path
  let path = url.pathname.slice('/sandbox/'.length);

  // Treat empty path or trailing slash as a directory -> index.html
  if (path === '' || path.endsWith('/')) {
    path += 'index.html';
  }

  const entry = fileCache.get(path);

  if (entry) {
    return new Response(entry.content, {
      status: 200,
      headers: { 'Content-Type': entry.mimeType },
    });
  }

  return new Response('Not Found', {
    status: 404,
    headers: { 'Content-Type': 'text/plain' },
  });
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
