/**
 * The sandbox bridge and its Service Worker must agree on their message
 * protocol version.
 *
 *   node --test tests/*.test.mjs
 *
 * The bridge (__lwid_sandbox__/sandbox.html) pings the worker controlling it
 * and only sends files once the worker answers with the bridge's own
 * SANDBOX_PROTOCOL. A worker left over from before an upgrade gets replaced
 * instead of being sent a file list it would misread. If the two constants
 * drifted, every load would wait for an update that never comes and fail.
 */
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const read = (rel) => readFileSync(fileURLToPath(new URL(rel, import.meta.url)), 'utf8');

function protocolIn(source, pattern, where) {
  const m = source.match(pattern);
  assert.ok(m, `no SANDBOX_PROTOCOL constant found in ${where}`);
  return Number(m[1]);
}

test('bridge and worker speak the same protocol version', () => {
  const worker = protocolIn(read('../shell/sandbox-sw.js'), /const SANDBOX_PROTOCOL = (\d+);/, 'sandbox-sw.js');
  const bridge = protocolIn(read('../shell/__lwid_sandbox__/sandbox.html'), /var SANDBOX_PROTOCOL = (\d+);/, 'sandbox.html');
  assert.equal(bridge, worker);
});

test('the worker answers PING with its protocol', () => {
  const worker = read('../shell/sandbox-sw.js');
  assert.match(worker, /type === 'PING'/);
  assert.match(worker, /type: 'PONG', protocol: SANDBOX_PROTOCOL/);
});
