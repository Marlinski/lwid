/**
 * viewers.js — Viewer detection for "lookwhatidid".
 *
 * A **viewer** is a small front-end shim the shell renders instead of a raw
 * file when a dropped project is not a website — a Jupyter notebook, a set of
 * Markdown docs, a bare folder of data files, and so on.
 *
 * Detection is a pure function of the project's file paths so it can be
 * unit-tested and evaluated before a single byte is decrypted. The shell then
 * asks the Service Worker to serve the matching bundle from `shell/viewers/`.
 *
 * Pure ES module, no dependencies.
 */

/**
 * Ordered viewer registry. The first entry whose `match(paths)` returns true
 * wins. `paths` is the list of project-relative file paths, lower-cased.
 *
 * @type {{ id: string, label: string, match: (paths: string[]) => boolean }[]}
 */
export const VIEWERS = [
  {
    id: 'notebook',
    label: 'Notebook',
    match: (paths) => paths.some((p) => p.endsWith('.ipynb')),
  },
  {
    id: 'markdown',
    label: 'Docs',
    match: (paths) => paths.some((p) => p.endsWith('.md') || p.endsWith('.markdown')),
  },
];

/** Viewer id used as the catch-all when nothing else matches. */
export const FALLBACK_VIEWER = 'files';

const HTML_RE = /\.html?$/;

/**
 * Pick a viewer for a project's file paths.
 *
 * Precedence:
 *   1. Any `.html` / `.htm` file  → `null` (render the site as-is; unchanged).
 *   2. `override`, when it names a known viewer (e.g. from project config).
 *   3. First matching entry in {@link VIEWERS}.
 *   4. {@link FALLBACK_VIEWER} — a browsable listing of whatever was uploaded.
 *
 * @param {Iterable<string>} paths     project-relative file paths
 * @param {string|null} [override]     explicit viewer id, or null
 * @returns {string|null} a viewer id, or `null` to use the default sandbox render
 */
export function detectViewer(paths, override = null) {
  const list = Array.from(paths || [], (p) => String(p).toLowerCase());
  if (list.length === 0) return null;

  if (list.some((p) => HTML_RE.test(p))) return null;

  if (override && knownViewer(override)) return override;

  for (const v of VIEWERS) {
    if (v.match(list)) return v.id;
  }

  return FALLBACK_VIEWER;
}

/** True if `id` is a viewer the shell knows how to serve. */
export function knownViewer(id) {
  return id === FALLBACK_VIEWER || VIEWERS.some((v) => v.id === id);
}
