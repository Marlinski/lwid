/**
 * page-link.js — the optional page part of a project link.
 *
 * A project link is `/p/{id}#{readKey}[:{writeKey}]`. A link to a page
 * inside the project appends that page after a `/`:
 *
 *   /p/{id}#{readKey}[:{writeKey}]/{path}[?query][#anchor]
 *
 * It lives in the fragment, like the keys, so the server never learns which
 * page (or which parameters) a link points to. Keys are base64url, which has
 * no `/`, so the first `/` always marks where the page starts, and older
 * links (no `/`) parse exactly as before.
 *
 * A page is `{ path, query }`: `path` is the file path inside the project,
 * decoded (`docs/my page.html`); `query` is the `?…` and/or `#…` part the
 * app's own URL had, kept verbatim, or `''`.
 */

/** Percent-encode each path segment, keeping `/` as the separator. */
function encodePath(path) {
  return String(path).split('/').map(encodeURIComponent).join('/');
}

/** Decode each path segment; a malformed escape is kept as written. */
function decodePath(path) {
  return path.split('/').map((seg) => {
    try { return decodeURIComponent(seg); } catch { return seg; }
  }).join('/');
}

/**
 * Split a link fragment into its keys and its optional page.
 *
 * @param {string} fragment — the URL hash, with or without its leading `#`
 * @returns {{ keys: string, page: { path: string, query: string } | null }}
 */
export function splitFragment(fragment) {
  const f = String(fragment).replace(/^#/, '');
  const slash = f.indexOf('/');
  if (slash === -1) return { keys: f, page: null };

  const rest = f.slice(slash + 1);
  const cut = rest.search(/[?#]/);
  const rawPath = cut === -1 ? rest : rest.slice(0, cut);
  const query = cut === -1 ? '' : rest.slice(cut);
  return { keys: f.slice(0, slash), page: { path: decodePath(rawPath), query } };
}

/**
 * The fragment suffix for a page: `''` for none, else `/{encoded path}{query}`.
 *
 * @param {{ path: string, query?: string } | null | undefined} page
 * @returns {string}
 */
export function pageSuffix(page) {
  if (!page) return '';
  return '/' + encodePath(page.path) + (page.query || '');
}

/**
 * True when `page` is just the project's first page: no page at all, or the
 * entry point (or the bare directory index) with no query and no anchor.
 * Sharing it needs no choice — it is the same as sharing the project.
 *
 * @param {{ path: string, query?: string } | null | undefined} page
 * @param {string | null} entryPoint — the resolved entry file, or null for a viewer project
 * @returns {boolean}
 */
export function isHomePage(page, entryPoint) {
  if (!page) return true;
  if (page.query) return false;
  return page.path === '' || page.path === entryPoint;
}

/**
 * How a page reads to a person: its path plus its query, e.g.
 * `docs/page2.html?id=3#intro`.
 */
export function describePage(page) {
  return page ? page.path + (page.query || '') : '';
}
