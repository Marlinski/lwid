/**
 * Saved-project list logic: lifetime formatting, expiry filtering, ordering.
 *
 * The browser only remembers a project's id and keys — the server holds the
 * timestamps. These helpers turn that pair into what the menu should show, and
 * are kept free of DOM and network so they can be tested directly.
 *
 * @module projects
 */

/** Metadata lookup result for a project the server no longer has. */
export const GONE = 'gone';

const MINUTE = 60 * 1000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

/**
 * Describe how much life a project has left.
 *
 * @param {string|null|undefined} expiresAt - ISO 8601 expiry, or null/undefined
 *   for a project that never expires.
 * @param {number} now - Current time in ms since epoch.
 * @returns {{ label: string, tone: 'permanent'|'expired'|'soon'|'ok' }}
 */
export function formatLifetime(expiresAt, now) {
  if (!expiresAt) return { label: 'permanent', tone: 'permanent' };

  const end = Date.parse(expiresAt);
  if (Number.isNaN(end)) return { label: 'permanent', tone: 'permanent' };

  const left = end - now;
  if (left <= 0) return { label: 'expired', tone: 'expired' };

  // Stay in hours right up to two days: flooring to days would report a
  // project with 47 hours left as "1d", which is the moment the number
  // matters most. Past that, round — a project created moments ago with a
  // 7-day TTL should read "7d left", not "6d".
  if (left < HOUR) {
    const m = Math.max(1, Math.floor(left / MINUTE));
    return { label: `${m}m left`, tone: 'soon' };
  }
  if (left < 2 * DAY) {
    const h = Math.floor(left / HOUR);
    return { label: `${h}h left`, tone: 'soon' };
  }
  const d = Math.round(left / DAY);
  return { label: `${d}d left`, tone: 'ok' };
}

/**
 * Decide which saved projects to show, in what order.
 *
 * Three states matter, and they are deliberately not treated alike:
 *
 * - **`GONE`** — the server returned 404. The project is unrecoverable, so it
 *   is dropped from the menu *and* reported for removal from storage.
 * - **expired** — still on the server but past its deadline; hidden, but left
 *   in storage because the reaper has not collected it yet.
 * - **unknown** — never fetched, or the request failed. Shown without a
 *   lifetime badge. A network blip must never make someone's projects vanish.
 *
 * Ordering is newest-created first, which keeps a project's position stable as
 * its remaining time ticks down. Entries whose creation time is unknown fall
 * back to when this browser last opened them.
 *
 * @param {{id: string, readKey: string, writeKey: string|null, lastVisited: number}[]} saved
 * @param {Record<string, {created_at?: string, expires_at?: string|null}|'gone'>} meta
 * @param {number} now
 * @returns {{ visible: object[], hiddenExpired: string[], drop: string[] }}
 */
export function selectVisibleProjects(saved, meta, now) {
  const visible = [];
  const hiddenExpired = [];
  const drop = [];

  for (const project of saved) {
    const info = meta[project.id];

    if (info === GONE) {
      drop.push(project.id);
      continue;
    }

    const lifetime = info ? formatLifetime(info.expires_at, now) : null;
    if (lifetime && lifetime.tone === 'expired') {
      hiddenExpired.push(project.id);
      continue;
    }

    visible.push({
      ...project,
      createdAt: info && info.created_at ? Date.parse(info.created_at) : null,
      lifetime,
    });
  }

  visible.sort((a, b) => {
    const at = a.createdAt ?? a.lastVisited ?? 0;
    const bt = b.createdAt ?? b.lastVisited ?? 0;
    return bt - at;
  });

  return { visible, hiddenExpired, drop };
}
