/**
 * Which TTL choices the current user may pick, and what unlocks the rest.
 *
 * The server is the authority: it clamps any requested TTL to the caller's
 * tier `max_ttl` (see `clamp_ttl` in crates/lwid-server/src/api/mod.rs).
 * This module only mirrors that rule so the shell can grey out the choices
 * the server would silently shorten, instead of letting the user pick one.
 */

/** Shortest to longest — must match `TTL_CHOICES` in crates/lwid-common/src/limits.rs. */
export const TTL_ORDER = ['1h', '1d', '7d', '30d', '180d', '365d', 'never'];

/**
 * The policy tier the server applies to this `/auth/me` response — mirrors
 * `tier_policy()`: no session is anonymous, `pro` is pro, anything else free.
 * @param {{id?: string, tier?: string} | null | undefined} me
 * @returns {'anonymous' | 'free' | 'pro'}
 */
export function tierOf(me) {
  if (!me || !me.id) return 'anonymous';
  return me.tier === 'pro' ? 'pro' : 'free';
}

/**
 * True when `ttl` is no longer than `maxTtl`. Anything outside
 * {@link TTL_ORDER} counts as allowed, matching the server, which leaves
 * unknown values unclamped.
 */
export function ttlAllowed(ttl, maxTtl) {
  const t = TTL_ORDER.indexOf(ttl);
  const m = TTL_ORDER.indexOf(maxTtl);
  return t === -1 || m === -1 || t <= m;
}

/**
 * Why a TTL choice is unavailable to this user, if it is.
 *
 * @param {string} ttl - the choice (`data-ttl` of its button)
 * @param {'anonymous' | 'free' | 'pro'} tier - the user's tier, from {@link tierOf}
 * @param {{anonymous: {max_ttl: string}, free: {max_ttl: string}, pro: {max_ttl: string}} | null} policy
 *   - the manifest's `policy` section
 * @param {boolean} authEnabled - whether the server offers sign-in at all
 * @returns {null | 'signin' | 'pro' | 'hidden'}
 *   `null` when allowed; `'signin'` when signing in to a free account
 *   unlocks it; `'pro'` when only a Pro account does; `'hidden'` when no
 *   account the user could get would allow it.
 */
export function ttlLock(ttl, tier, policy, authEnabled) {
  if (!policy || !policy[tier]) return null;
  if (ttlAllowed(ttl, policy[tier].max_ttl)) return null;
  // Without sign-in nobody can move to another tier, so a lock would only
  // advertise something the user can never get.
  if (!authEnabled) return 'hidden';
  if (tier === 'anonymous' && ttlAllowed(ttl, policy.free.max_ttl)) return 'signin';
  if (tier !== 'pro' && ttlAllowed(ttl, policy.pro.max_ttl)) return 'pro';
  return 'hidden';
}
