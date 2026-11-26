/**
 * Tests for lazy file loading (shell/js/project.js).
 *
 *   node --test tests/
 */
import test from 'node:test';
import assert from 'node:assert/strict';

import { generateReadKey, generateWriteKeyPair, encrypt } from '../shell/js/crypto.js';
import { computeCid } from '../shell/js/cid.js';
import { createManifest, serializeManifest } from '../shell/js/manifest.js';
import { pullVersion, pushVersion, ProjectFiles } from '../shell/js/project.js';

/** In-memory stand-in for ApiClient that counts blob reads. */
function fakeApi() {
  const blobs = new Map();
  const api = {
    blobs,
    reads: [],
    root: null,
    async getBlob(cid) {
      api.reads.push(cid);
      if (!blobs.has(cid)) throw new Error(`no blob ${cid}`);
      return blobs.get(cid).slice().buffer;
    },
    async blobExists(cid) { return blobs.has(cid); },
    async uploadBlob(bytes) { blobs.set(await computeCid(bytes), bytes); },
    async getProject() { return { root_cid: api.root }; },
    async updateRoot(_id, cid) { api.root = cid; },
  };
  return api;
}

const enc = (s) => new TextEncoder().encode(s);
const dec = (b) => new TextDecoder().decode(b);

async function publish(api, readKey, files) {
  const entries = [];
  for (const f of files) {
    const bytes = await encrypt(readKey, enc(f.content));
    await api.uploadBlob(bytes);
    entries.push({ path: f.path, cid: await computeCid(bytes), size: enc(f.content).byteLength });
  }
  const manifestBytes = serializeManifest(await createManifest(entries, null, readKey));
  await api.uploadBlob(manifestBytes);
  return computeCid(manifestBytes);
}

test('pullVersion fetches only the manifest, no file contents', async () => {
  const api = fakeApi();
  const key = generateReadKey();
  const manifestCid = await publish(api, key, [
    { path: 'a.wav', content: 'AAAA' },
    { path: 'b.wav', content: 'BBBBBB' },
  ]);
  api.reads.length = 0;

  const { files } = await pullVersion(api, key, manifestCid);

  assert.deepEqual(api.reads, [manifestCid]);
  assert.equal(files.size, 2);
  assert.ok(files.has('a.wav'));
  assert.deepEqual([...files.keys()].sort(), ['a.wav', 'b.wav']);
  assert.equal(files.sizeOf('b.wav'), 6);
});

test('get() decrypts one file on demand and caches it', async () => {
  const api = fakeApi();
  const key = generateReadKey();
  const manifestCid = await publish(api, key, [
    { path: 'a.wav', content: 'AAAA' },
    { path: 'b.wav', content: 'BBBBBB' },
  ]);
  const { files } = await pullVersion(api, key, manifestCid);
  api.reads.length = 0;

  const [first, second] = await Promise.all([files.get('a.wav'), files.get('a.wav')]);
  assert.equal(dec(first), 'AAAA');
  assert.equal(second, first);
  assert.equal(dec(await files.get('a.wav')), 'AAAA');
  assert.deepEqual(api.reads, [files.entry('a.wav').cid]);
});

test('get() rejects an unknown path', async () => {
  const files = new ProjectFiles(fakeApi(), generateReadKey(), []);
  await assert.rejects(files.get('nope.wav'), /nope\.wav/);
});

test('a failed load is not cached', async () => {
  const api = fakeApi();
  const key = generateReadKey();
  const manifestCid = await publish(api, key, [{ path: 'a.wav', content: 'AAAA' }]);
  const { files } = await pullVersion(api, key, manifestCid);
  const cid = files.entry('a.wav').cid;
  const saved = api.blobs.get(cid);
  api.blobs.delete(cid);

  await assert.rejects(files.get('a.wav'));
  api.blobs.set(cid, saved);
  assert.equal(dec(await files.get('a.wav')), 'AAAA');
});

test('pushVersion reuses the blob of a file given by cid without reading it', async () => {
  const api = fakeApi();
  const key = generateReadKey();
  api.root = await publish(api, key, [
    { path: 'big.wav', content: 'WAVDATA' },
    { path: 'notes.txt', content: 'old' },
  ]);
  const { files } = await pullVersion(api, key, api.root);
  api.reads.length = 0;

  const { privateKeyB64url: writeKey } = await generateWriteKeyPair();
  const { files: pushed } = await pushVersion(api, 'p', key, writeKey, [
    files.entry('big.wav'),
    { path: 'notes.txt', content: enc('new') },
  ]);

  assert.deepEqual(api.reads, []);
  const big = pushed.find((f) => f.path === 'big.wav');
  assert.deepEqual(big, files.entry('big.wav'));

  const { files: next } = await pullVersion(api, key, api.root);
  assert.equal(dec(await next.get('big.wav')), 'WAVDATA');
  assert.equal(dec(await next.get('notes.txt')), 'new');
});
