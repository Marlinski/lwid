/**
 * Tests for the saved-project list logic (shell/js/projects.js).
 *
 * Uses the Node built-in test runner — no dependencies, no build step:
 *
 *   node --test tests/
 */
import test from 'node:test';
import assert from 'node:assert/strict';

import { formatLifetime, selectVisibleProjects, GONE } from '../shell/js/projects.js';

const NOW = Date.parse('2026-09-01T12:00:00Z');
const MIN = 60_000;
const HR = 60 * MIN;
const DAY = 24 * HR;
const at = (offset) => new Date(NOW + offset).toISOString();

test('formatLifetime: a project with no deadline is permanent', () => {
  assert.deepEqual(formatLifetime(null, NOW), { label: 'permanent', tone: 'permanent' });
  assert.deepEqual(formatLifetime(undefined, NOW), { label: 'permanent', tone: 'permanent' });
});

test('formatLifetime: an unparseable date degrades to permanent rather than expired', () => {
  // Mislabelling a live project as expired would hide it, which is the one
  // outcome worse than showing no countdown at all.
  assert.deepEqual(formatLifetime('not-a-date', NOW), { label: 'permanent', tone: 'permanent' });
});

test('formatLifetime: the deadline itself counts as expired', () => {
  assert.deepEqual(formatLifetime(at(-1), NOW), { label: 'expired', tone: 'expired' });
  assert.deepEqual(formatLifetime(at(0), NOW), { label: 'expired', tone: 'expired' });
});

test('formatLifetime: under an hour reports minutes, never "0m"', () => {
  assert.deepEqual(formatLifetime(at(30 * MIN), NOW), { label: '30m left', tone: 'soon' });
  assert.deepEqual(formatLifetime(at(30_000), NOW), { label: '1m left', tone: 'soon' });
});

test('formatLifetime: hours are kept right up to two days', () => {
  // "1d left" with 47 hours to go is least useful exactly when it matters.
  assert.deepEqual(formatLifetime(at(5 * HR + 59 * MIN), NOW), { label: '5h left', tone: 'soon' });
  assert.deepEqual(formatLifetime(at(25 * HR), NOW), { label: '25h left', tone: 'soon' });
  assert.deepEqual(formatLifetime(at(47 * HR), NOW), { label: '47h left', tone: 'soon' });
});

test('formatLifetime: a freshly created 7d project reads "7d left", not "6d"', () => {
  assert.deepEqual(formatLifetime(at(7 * DAY - HR), NOW), { label: '7d left', tone: 'ok' });
  assert.deepEqual(formatLifetime(at(7 * DAY), NOW), { label: '7d left', tone: 'ok' });
  assert.deepEqual(formatLifetime(at(50 * HR), NOW), { label: '2d left', tone: 'ok' });
  assert.deepEqual(formatLifetime(at(30 * DAY), NOW), { label: '30d left', tone: 'ok' });
});

const saved = [
  { id: 'aaa', readKey: 'r', writeKey: 'w', lastVisited: 1 },
  { id: 'bbb', readKey: 'r', writeKey: null, lastVisited: 2 },
  { id: 'ccc', readKey: 'r', writeKey: null, lastVisited: 3 },
  { id: 'ddd', readKey: 'r', writeKey: null, lastVisited: 4 },
];
const meta = {
  aaa: { created_at: at(-10 * DAY), expires_at: at(3 * DAY) },
  bbb: { created_at: at(-20 * DAY), expires_at: at(-1 * DAY) },
  ccc: GONE,
  ddd: { created_at: at(-1 * DAY), expires_at: null },
};

test('selectVisibleProjects: hides expired and missing, newest created first', () => {
  const { visible } = selectVisibleProjects(saved, meta, NOW);
  assert.deepEqual(visible.map((p) => p.id), ['ddd', 'aaa']);
  assert.equal(visible[0].lifetime.label, 'permanent');
  assert.equal(visible[1].lifetime.label, '3d left');
});

test('selectVisibleProjects: only a 404 removes an entry from storage', () => {
  const { hiddenExpired, drop } = selectVisibleProjects(saved, meta, NOW);
  // Expired is hidden but retained — the server may not have reaped it yet,
  // and the entry still carries the only copy of the keys.
  assert.deepEqual(hiddenExpired, ['bbb']);
  assert.deepEqual(drop, ['ccc']);
});

test('selectVisibleProjects: unknown metadata never hides a project', () => {
  // A failed request must not look like an expiry.
  const { visible, drop } = selectVisibleProjects(saved, {}, NOW);
  assert.deepEqual(visible.map((p) => p.id), ['ddd', 'ccc', 'bbb', 'aaa']);
  assert.ok(visible.every((p) => p.lifetime === null));
  assert.deepEqual(drop, []);
});

test('selectVisibleProjects: falls back to last-visited order without creation times', () => {
  const { visible } = selectVisibleProjects(saved, {}, NOW);
  assert.deepEqual(visible.map((p) => p.lastVisited), [4, 3, 2, 1]);
});

test('selectVisibleProjects: partially loaded metadata still filters what it knows', () => {
  const { visible } = selectVisibleProjects(saved, { bbb: meta.bbb }, NOW);
  assert.deepEqual(visible.map((p) => p.id), ['ddd', 'ccc', 'aaa']);
});

test('selectVisibleProjects: empty list', () => {
  assert.deepEqual(selectVisibleProjects([], {}, NOW).visible, []);
});
