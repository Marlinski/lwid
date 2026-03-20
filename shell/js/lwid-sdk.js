/**
 * lwid SDK — Persistent storage for apps hosted on lookwhatidid.ovh
 *
 * This script is automatically injected into every HTML page served in the
 * sandbox. It provides two storage APIs via `window.lwid`:
 *
 *   - `lwid.store`  — JSON key-value store (auto serialize/deserialize)
 *   - `lwid.blobs`  — Binary blob store (ArrayBuffer values)
 *
 * All data is encrypted client-side with AES-256-GCM before reaching the
 * server. The encryption key is derived from the project URL — you don't
 * need to handle encryption yourself.
 *
 * **Limits:** 10 MB per value, 50 MB total per project.
 *
 * ## Quick start
 *
 *   // Store a JSON value
 *   await lwid.store.set('score', { player: 'Alice', points: 42 });
 *
 *   // Read it back
 *   const score = await lwid.store.get('score');
 *   console.log(score.points); // 42
 *
 *   // Store a binary file
 *   const response = await fetch('photo.png');
 *   await lwid.blobs.set('photo', await response.arrayBuffer());
 *
 *   // Read it back
 *   const buf = await lwid.blobs.get('photo');
 *   const url = URL.createObjectURL(new Blob([buf]));
 *
 * ## How it works
 *
 * The SDK communicates with the parent shell (lookwhatidid.ovh) via
 * `window.parent.postMessage`. Each call sends a typed request with a unique
 * ID and waits for a `LWID_RESULT` message carrying the same ID. The parent
 * shell handles encryption, persistence, and quota enforcement transparently.
 *
 * If the parent shell does not respond within 10 seconds, the promise rejects
 * with a timeout error.
 *
 * @global
 * @namespace lwid
 */
(function () {
  'use strict';
  if (window.lwid) return; // already injected

  /** @type {number} Auto-incrementing request ID counter. */
  let nextId = 1;

  /**
   * Map of pending request IDs to their Promise settle callbacks.
   * @type {Map<number, { resolve: Function, reject: Function }>}
   */
  const pending = new Map();

  /** Timeout in milliseconds before a request is considered failed. */
  const REQUEST_TIMEOUT_MS = 10000;

  // ---------------------------------------------------------------------------
  // postMessage listener — resolves / rejects pending promises
  // ---------------------------------------------------------------------------

  window.addEventListener('message', (event) => {
    const msg = event.data;
    if (!msg || msg.type !== 'LWID_RESULT') return;
    const entry = pending.get(msg.id);
    if (!entry) return;
    pending.delete(msg.id);
    if (msg.error) {
      entry.reject(new Error(msg.error));
    } else {
      entry.resolve(msg.value);
    }
  });

  // ---------------------------------------------------------------------------
  // Internal request helper
  // ---------------------------------------------------------------------------

  /**
   * Send a typed request to the parent shell and return a promise that resolves
   * with the response value.
   *
   * The promise rejects if the parent does not respond within
   * {@link REQUEST_TIMEOUT_MS} milliseconds or if the parent returns an error.
   *
   * @param {string}  type     The postMessage request type (e.g. `'LWID_STORE_GET'`).
   * @param {Object}  payload  Additional fields merged into the message.
   * @returns {Promise<*>} The value returned by the parent shell.
   */
  function request(type, payload) {
    return new Promise((resolve, reject) => {
      const id = nextId++;
      const timer = setTimeout(() => {
        pending.delete(id);
        reject(new Error(`lwid SDK: request ${type} timed out after ${REQUEST_TIMEOUT_MS / 1000}s`));
      }, REQUEST_TIMEOUT_MS);
      pending.set(id, {
        resolve: (v) => { clearTimeout(timer); resolve(v); },
        reject:  (e) => { clearTimeout(timer); reject(e); },
      });
      window.parent.postMessage({ type, id, ...payload }, '*');
    });
  }

  // ---------------------------------------------------------------------------
  // Public API
  // ---------------------------------------------------------------------------

  window.lwid = {
    /**
     * JSON key-value store.
     *
     * Values are automatically JSON-serialized on write and JSON-deserialized
     * on read — you can store any JSON-serializable value (objects, arrays,
     * strings, numbers, booleans, `null`).
     *
     * All values are encrypted client-side before storage. Each value may be
     * up to 10 MB; total storage per project is capped at 50 MB.
     *
     * @namespace lwid.store
     */
    store: {
      /**
       * Retrieve a value by key.
       *
       * @param   {string} key  The storage key.
       * @returns {Promise<*>}  The deserialized value, or `null` if the key
       *                        does not exist.
       * @example
       * const score = await lwid.store.get('score');
       * console.log(score); // { player: 'Alice', points: 42 }
       */
      async get(key) {
        const raw = await request('LWID_STORE_GET', { key });
        return raw === null || raw === undefined ? null : JSON.parse(raw);
      },

      /**
       * Store a value under the given key. Any existing value is overwritten.
       *
       * The value is JSON-serialized automatically — pass any
       * JSON-serializable value (object, array, string, number, boolean, or
       * `null`).
       *
       * @param   {string} key    The storage key.
       * @param   {*}      value  Any JSON-serializable value.
       * @returns {Promise<void>}
       * @example
       * await lwid.store.set('score', { player: 'Alice', points: 42 });
       * await lwid.store.set('greeting', 'hello world');
       */
      async set(key, value) {
        await request('LWID_STORE_SET', { key, value: JSON.stringify(value) });
      },

      /**
       * Delete a key and its value from the store.
       *
       * @param   {string} key  The storage key to remove.
       * @returns {Promise<void>}
       * @example
       * await lwid.store.delete('score');
       */
      async delete(key) {
        await request('LWID_STORE_DELETE', { key });
      },

      /**
       * List all keys currently stored in the JSON store.
       *
       * @returns {Promise<string[]>}  An array of key strings.
       * @example
       * const keys = await lwid.store.keys();
       * console.log(keys); // ['score', 'greeting']
       */
      async keys() {
        return request('LWID_STORE_KEYS', {});
      },
    },

    /**
     * Binary blob store.
     *
     * Values are raw `ArrayBuffer` instances — use this for images, audio,
     * binary files, or any data that isn't plain JSON.
     *
     * All blobs are encrypted client-side before storage. Each blob may be
     * up to 10 MB; total storage per project is capped at 50 MB.
     *
     * @namespace lwid.blobs
     */
    blobs: {
      /**
       * Retrieve a binary blob by key.
       *
       * @param   {string} key  The blob key.
       * @returns {Promise<ArrayBuffer|null>}  The raw binary data, or `null`
       *                                       if the key does not exist.
       * @example
       * const buf = await lwid.blobs.get('photo');
       * const url = URL.createObjectURL(new Blob([buf], { type: 'image/png' }));
       */
      async get(key) {
        return request('LWID_BLOB_GET', { key });
      },

      /**
       * Store a binary blob under the given key. Any existing blob is
       * overwritten.
       *
       * @param   {string}      key    The blob key.
       * @param   {ArrayBuffer} value  The raw binary data to store.
       * @returns {Promise<void>}
       * @example
       * const response = await fetch('photo.png');
       * await lwid.blobs.set('photo', await response.arrayBuffer());
       */
      async set(key, value) {
        await request('LWID_BLOB_SET', { key, value });
      },

      /**
       * Delete a blob by key.
       *
       * @param   {string} key  The blob key to remove.
       * @returns {Promise<void>}
       * @example
       * await lwid.blobs.delete('photo');
       */
      async delete(key) {
        await request('LWID_BLOB_DELETE', { key });
      },

      /**
       * List all keys currently stored in the blob store.
       *
       * @returns {Promise<string[]>}  An array of key strings.
       * @example
       * const keys = await lwid.blobs.list();
       * console.log(keys); // ['photo', 'backup.zip']
       */
      async list() {
        return request('LWID_BLOB_LIST', {});
      },
    },
  };
})();
