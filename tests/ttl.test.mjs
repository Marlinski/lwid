/**
 * Tests for the TTL lock logic (shell/js/ttl.js).
 *
 *   node --test tests/
 */
import test from 'node:test';
import assert from 'node:assert/strict';

import { TTL_ORDER, tierOf, ttlAllowed, ttlLock } from '../shell/js/ttl.js';

// The server's default tier limits (crates/lwid-server/src/config.rs).
const POLICY = {
  anonymous: { max_ttl: '7d' },
  free: { max_ttl: '30d' },
  pro: { max_ttl: 'never' },
};

const locks = (tier, policy = POLICY, authEnabled = true) =>
  Object.fromEntries(TTL_ORDER.map((t) => [t, ttlLock(t, tier, policy, authEnabled)]));

test('tierOf mirrors the server: no session is anonymous, pro is pro, the rest free', () => {
  assert.equal(tierOf(null), 'anonymous');
  assert.equal(tierOf({ id: '', tier: 'anonymous' }), 'anonymous');
  assert.equal(tierOf({ id: 'u1', tier: 'free' }), 'free');
  assert.equal(tierOf({ id: 'u1', tier: 'pro' }), 'pro');
  assert.equal(tierOf({ id: 'u1', tier: 'something-new' }), 'free');
});

test('ttlAllowed follows TTL_ORDER and lets unknown values through', () => {
  assert.equal(ttlAllowed('30d', '30d'), true);
  assert.equal(ttlAllowed('180d', '30d'), false);
  assert.equal(ttlAllowed('365d', 'never'), true);
  assert.equal(ttlAllowed('never', '365d'), false);
  assert.equal(ttlAllowed('2w', '1d'), true);
});

test('anonymous: sign in unlocks 30d, the longer choices need Pro', () => {
  assert.deepEqual(locks('anonymous'), {
    '1h': null, '1d': null, '7d': null,
    '30d': 'signin', '180d': 'pro', '365d': 'pro', never: 'pro',
  });
});

test('free: everything past 30d needs Pro', () => {
  assert.deepEqual(locks('free'), {
    '1h': null, '1d': null, '7d': null, '30d': null,
    '180d': 'pro', '365d': 'pro', never: 'pro',
  });
});

test('pro: nothing is locked', () => {
  assert.ok(Object.values(locks('pro')).every((l) => l === null));
});

test('a choice no tier allows is hidden, not advertised as Pro', () => {
  const capped = { ...POLICY, pro: { max_ttl: '365d' } };
  assert.equal(ttlLock('never', 'free', capped, true), 'hidden');
  assert.equal(ttlLock('never', 'pro', capped, true), 'hidden');
});

test('without sign-in, locked choices are hidden since no upgrade is possible', () => {
  assert.equal(ttlLock('30d', 'anonymous', POLICY, false), 'hidden');
  assert.equal(ttlLock('7d', 'anonymous', POLICY, false), null);
});

test('no policy known yet: lock nothing and let the server decide', () => {
  assert.equal(ttlLock('never', 'anonymous', null, true), null);
});
