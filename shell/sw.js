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
 *
 * Re-hydration:
 *   The file cache is in-memory, so it is wiped whenever the browser
 *   terminates this (idle) Service Worker. On a subsequent /sandbox/ request
 *   the cache is empty and every navigation would 404 until the shell page is
 *   reloaded. To avoid that, an empty-cache miss asks a live shell client to
 *   re-send its (already-decrypted, in-memory) files via SET_FILES, then the
 *   request is retried — so intra-app navigation keeps working across SW
 *   restarts without ever persisting plaintext to disk.
 */

// ---------------------------------------------------------------------------
// In-memory file cache: path -> { content: Uint8Array, mimeType: string }
// ---------------------------------------------------------------------------
const fileCache = new Map();

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
    // Unblock any /sandbox/ request that was waiting on re-hydration.
    finishHydration();
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

async function handleSandboxRequest(url) {
  // Strip the /sandbox/ prefix to derive the file path
  let path = url.pathname.slice('/sandbox/'.length);

  // Treat empty path or trailing slash as a directory -> index.html
  if (path === '' || path.endsWith('/')) {
    path += 'index.html';
  }

  let entry = fileCache.get(path);

  // Empty cache ⇒ the SW was almost certainly restarted after going idle.
  // Ask the shell to re-send the files, then retry once. (A non-empty cache
  // that simply lacks this path is a genuine 404 — don't re-hydrate.)
  if (!entry && fileCache.size === 0) {
    await requestHydration();
    entry = fileCache.get(path);
  }

  if (entry) {
    // Inject lwid-sdk.js into HTML responses
    if (entry.mimeType === 'text/html') {
      let html = new TextDecoder().decode(entry.content);
      const scriptTag = '<script src="/js/lwid-sdk.js"></script>';

      const headMatch = html.match(/<head(\s[^>]*)?>|<head>/i);
      if (headMatch) {
        // Insert right after the opening <head...> tag
        const insertPos = headMatch.index + headMatch[0].length;
        html = html.slice(0, insertPos) + scriptTag + html.slice(insertPos);
      } else {
        const htmlMatch = html.match(/<html(\s[^>]*)?>|<html>/i);
        if (htmlMatch) {
          // Insert <head> block with script after <html...>
          const insertPos = htmlMatch.index + htmlMatch[0].length;
          html = html.slice(0, insertPos) + '<head>' + scriptTag + '</head>' + html.slice(insertPos);
        } else {
          // No <head> or <html> tag — prepend to document
          html = scriptTag + '\n' + html;
        }
      }

      return new Response(html, {
        status: 200,
        headers: { 'Content-Type': entry.mimeType },
      });
    }

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
