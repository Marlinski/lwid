/**
 * viewer-host.js — bridge between a viewer bundle and the lwid shell.
 *
 * Loaded as a plain <script> by every viewer (`/sandbox/__shared__/viewer-host.js`).
 * It exposes `window.LwidHost`:
 *
 *   await LwidHost.manifest()              -> { files, viewer, canEdit }
 *   await LwidHost.files()                 -> [{ path, size, mimeType }]
 *   await LwidHost.canEdit()               -> boolean
 *   await LwidHost.readBytes(path)         -> Uint8Array
 *   await LwidHost.readText(path)          -> string
 *   await LwidHost.readJSON(path)          -> any
 *   await LwidHost.saveVersion(changed)    -> { manifestCid }
 *       changed: [{ path, content: string | Uint8Array | ArrayBuffer }]
 *
 * File reads go straight through the Service Worker (`/sandbox/<path>`); only
 * `saveVersion` needs the shell, which owns the write key and the push flow.
 * That round-trip uses `LWID_HOST_*` messages, kept separate from the
 * `LWID_*` store protocol in lwid-sdk.js so the two never collide on ids.
 */
(function () {
  'use strict';
  if (window.LwidHost) return;

  const pending = new Map();
  let nextId = 1;
  let manifestCache = null;

  window.addEventListener('message', (event) => {
    const msg = event.data;
    if (!msg || msg.type !== 'LWID_HOST_RESULT') return;
    const entry = pending.get(msg.id);
    if (!entry) return;
    pending.delete(msg.id);
    if (msg.error) entry.reject(new Error(msg.error));
    else entry.resolve(msg.value);
  });

  function request(type, payload, timeoutMs = 60000) {
    return new Promise((resolve, reject) => {
      const id = nextId++;
      const timer = setTimeout(() => {
        pending.delete(id);
        reject(new Error(`LwidHost: ${type} timed out`));
      }, timeoutMs);
      pending.set(id, {
        resolve: (v) => { clearTimeout(timer); resolve(v); },
        reject: (e) => { clearTimeout(timer); reject(e); },
      });
      window.parent.postMessage({ type, id, ...payload }, '*');
    });
  }

  function encodePath(path) {
    return path.split('/').map(encodeURIComponent).join('/');
  }

  async function manifest(force) {
    if (manifestCache && !force) return manifestCache;
    const res = await fetch('/sandbox/__lwid/files.json', { cache: 'no-store' });
    if (!res.ok) throw new Error(`LwidHost: manifest fetch failed (${res.status})`);
    manifestCache = await res.json();
    return manifestCache;
  }

  async function readBytes(path) {
    const res = await fetch('/sandbox/' + encodePath(path));
    if (!res.ok) throw new Error(`LwidHost: cannot read ${path} (${res.status})`);
    return new Uint8Array(await res.arrayBuffer());
  }

  function toArrayBuffer(content) {
    if (typeof content === 'string') return new TextEncoder().encode(content).buffer;
    if (content instanceof Uint8Array) {
      return content.byteOffset === 0 && content.byteLength === content.buffer.byteLength
        ? content.buffer
        : content.slice().buffer;
    }
    if (content instanceof ArrayBuffer) return content;
    throw new TypeError('LwidHost.saveVersion: content must be string, Uint8Array or ArrayBuffer');
  }

  window.LwidHost = {
    manifest,
    async files() { return (await manifest()).files; },
    async canEdit() { return !!(await manifest()).canEdit; },
    async viewerId() { return (await manifest()).viewer; },
    readBytes,
    async readText(path) { return new TextDecoder().decode(await readBytes(path)); },
    async readJSON(path) { return JSON.parse(await this.readText(path)); },
    async saveVersion(changed) {
      const files = (changed || []).map((f) => ({
        path: f.path,
        content: toArrayBuffer(f.content),
      }));
      if (files.length === 0) throw new Error('LwidHost.saveVersion: nothing to save');
      const result = await request('LWID_HOST_SAVE', { files }, 120000);
      manifestCache = null; // project changed underneath us
      return result;
    },
  };
})();
