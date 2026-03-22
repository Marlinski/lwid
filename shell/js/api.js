/**
 * Browser-side HTTP API client for lookwhatidid.
 * Wraps all calls to the server API using the Fetch API.
 * @module api
 */

/**
 * Error thrown when the server returns a non-2xx response.
 */
export class ApiError extends Error {
  /**
   * @param {number} status - HTTP status code
   * @param {string} message - Error message (from server or default)
   */
  constructor(status, message) {
    super(message);
    this.name = 'ApiError';
    /** @type {number} */
    this.status = status;
  }
}

/**
 * Parse a non-2xx response into an ApiError.
 * Attempts to read a JSON body with an `error` field; falls back to status text.
 *
 * @param {Response} res
 * @returns {Promise<ApiError>}
 */
async function errorFromResponse(res) {
  let message = res.statusText;
  try {
    const body = await res.json();
    if (body && typeof body.error === 'string') {
      message = body.error;
    }
  } catch {
    // Body wasn't JSON — keep statusText.
  }
  return new ApiError(res.status, message);
}

/**
 * HTTP API client for the lookwhatidid server.
 */
export class ApiClient {
  /**
   * @param {string} baseUrl - Server origin (e.g. "" for same-origin,
   *   or "https://lookwhatidid.xyz").
   */
  constructor(baseUrl = '') {
    /** @type {string} */
    this.baseUrl = baseUrl.replace(/\/+$/, '');
  }

  // ---------------------------------------------------------------------------
  // Projects
  // ---------------------------------------------------------------------------

  /**
   * Create a new project.
   *
   * @param {string} writePublicKeyBase64 - Base-64-encoded public key that is
   *   authorised to update the project root.
   * @returns {Promise<{ project_id: string }>}
   * @throws {ApiError}
   */
  async createProject(writePublicKeyBase64, ttl = '7d', storeToken = null) {
    const body = { write_pubkey: writePublicKeyBase64, ttl };
    if (storeToken) body.store_token = storeToken;
    const res = await fetch(`${this.baseUrl}/api/projects`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    });
    if (!res.ok) throw await errorFromResponse(res);
    return res.json();
  }

  /**
   * Retrieve a project by ID.
   *
   * @param {string} id - Project identifier.
   * @returns {Promise<{ id: string, root_cid: string|null, created_at: string, updated_at: string }>}
   * @throws {ApiError}
   */
  async getProject(id) {
    const res = await fetch(`${this.baseUrl}/api/projects/${encodeURIComponent(id)}`);
    if (!res.ok) throw await errorFromResponse(res);
    return res.json();
  }

  /**
   * Update the root CID of a project (requires a valid signature).
   *
   * @param {string} id - Project identifier.
   * @param {string} rootCid - New root CID.
   * @param {string} signatureBase64 - Base-64-encoded signature over the new
   *   root CID, produced with the project's write key.
   * @returns {Promise<{ id: string, root_cid: string, created_at: string, updated_at: string }>}
   * @throws {ApiError}
   */
  async updateRoot(id, rootCid, signatureBase64) {
    const res = await fetch(`${this.baseUrl}/api/projects/${encodeURIComponent(id)}/root`, {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ root_cid: rootCid, signature: signatureBase64 }),
    });
    if (!res.ok) throw await errorFromResponse(res);
    return res.json();
  }

  // ---------------------------------------------------------------------------
  // Blobs
  // ---------------------------------------------------------------------------

  /**
   * Upload a binary blob.
   *
   * @param {Uint8Array} uint8Array - Raw bytes to store.
   * @returns {Promise<{ cid: string }>}
   * @throws {ApiError}
   */
  async uploadBlob(uint8Array) {
    const res = await fetch(`${this.baseUrl}/api/blobs`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/octet-stream' },
      body: uint8Array,
    });
    if (!res.ok) throw await errorFromResponse(res);
    return res.json();
  }

  /**
   * Download a blob by its CID.
   *
   * @param {string} cid - Content identifier of the blob.
   * @returns {Promise<Uint8Array>}
   * @throws {ApiError}
   */
  async getBlob(cid) {
    const res = await fetch(`${this.baseUrl}/api/blobs/${encodeURIComponent(cid)}`);
    if (!res.ok) throw await errorFromResponse(res);
    const buf = await res.arrayBuffer();
    return new Uint8Array(buf);
  }

  /**
   * Extend (or set) the TTL of a project.
   *
   * @param {string} id - Project identifier.
   * @param {string} ttl - TTL string (e.g. "1h", "1d", "7d", "30d", "never").
   * @param {string} signatureBase64 - Base-64-encoded signature over the TTL
   *   string, produced with the project's write key.
   * @returns {Promise<{ id: string, expires_at: string|null }>}
   * @throws {ApiError}
   */
  async extendTtl(id, ttl, signatureBase64) {
    const res = await fetch(`${this.baseUrl}/api/projects/${encodeURIComponent(id)}/ttl`, {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ ttl, signature: signatureBase64 }),
    });
    if (!res.ok) throw await errorFromResponse(res);
    return res.json();
  }

  /**
   * Check whether a blob exists on the server.
   *
   * @param {string} cid - Content identifier of the blob.
   * @returns {Promise<boolean>} `true` if the blob exists (HTTP 200),
   *   `false` if not found (HTTP 404).
   * @throws {ApiError} For any status other than 200 or 404.
   */
  async blobExists(cid) {
    const res = await fetch(`${this.baseUrl}/api/blobs/${encodeURIComponent(cid)}`, {
      method: 'HEAD',
    });
    if (res.ok) return true;
    if (res.status === 404) return false;
    throw await errorFromResponse(res);
  }

  // ---------------------------------------------------------------------------
  // Store
  // ---------------------------------------------------------------------------

  /**
   * Store a value under the given key.
   *
   * @param {string} id - Project identifier.
   * @param {string} key - Store key (may contain `/` for hierarchical keys).
   * @param {Uint8Array} value - Raw bytes to store.
   * @param {string} storeToken - Token sent as `X-Store-Token` header.
   * @returns {Promise<void>}
   * @throws {ApiError}
   */
  async putStoreValue(id, key, value, storeToken) {
    const res = await fetch(`${this.baseUrl}/api/projects/${encodeURIComponent(id)}/store/${key}`, {
      method: 'PUT',
      headers: {
        'Content-Type': 'application/octet-stream',
        'X-Store-Token': storeToken,
      },
      body: value,
    });
    if (!res.ok) throw await errorFromResponse(res);
  }

  /**
   * Retrieve the value stored under the given key.
   *
   * @param {string} id - Project identifier.
   * @param {string} key - Store key (may contain `/` for hierarchical keys).
   * @param {string} storeToken - Token sent as `X-Store-Token` header.
   * @returns {Promise<Uint8Array|null>} The raw bytes, or `null` if the key
   *   does not exist (HTTP 404).
   * @throws {ApiError} For any status other than 200 or 404.
   */
  async getStoreValue(id, key, storeToken) {
    const res = await fetch(`${this.baseUrl}/api/projects/${encodeURIComponent(id)}/store/${key}`, {
      headers: { 'X-Store-Token': storeToken },
    });
    if (res.status === 404) return null;
    if (!res.ok) throw await errorFromResponse(res);
    const buf = await res.arrayBuffer();
    return new Uint8Array(buf);
  }

  /**
   * Delete the value stored under the given key.
   *
   * @param {string} id - Project identifier.
   * @param {string} key - Store key (may contain `/` for hierarchical keys).
   * @param {string} storeToken - Token sent as `X-Store-Token` header.
   * @returns {Promise<void>}
   * @throws {ApiError}
   */
  async deleteStoreValue(id, key, storeToken) {
    const res = await fetch(`${this.baseUrl}/api/projects/${encodeURIComponent(id)}/store/${key}`, {
      method: 'DELETE',
      headers: { 'X-Store-Token': storeToken },
    });
    if (!res.ok) throw await errorFromResponse(res);
  }

  /**
   * List all keys in the project store.
   *
   * @param {string} id - Project identifier.
   * @param {string} storeToken - Token sent as `X-Store-Token` header.
   * @returns {Promise<{ keys: Array<{key: string, size: number}>, total_size: number }>}
   * @throws {ApiError}
   */
  async listStoreKeys(id, storeToken) {
    const res = await fetch(`${this.baseUrl}/api/projects/${encodeURIComponent(id)}/store`, {
      headers: { 'X-Store-Token': storeToken },
    });
    if (!res.ok) throw await errorFromResponse(res);
    return res.json();
  }

  /**
   * Delete a project. Requires a signature over the project ID.
   *
   * @param {string} id - Project identifier.
   * @param {string} signatureBase64 - Base-64-encoded signature over the
   *   project ID string, produced with the project's write key.
   * @returns {Promise<void>}
   * @throws {ApiError}
   */
  async deleteProject(id, signatureBase64) {
    const res = await fetch(`${this.baseUrl}/api/projects/${encodeURIComponent(id)}`, {
      method: 'DELETE',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ signature: signatureBase64 }),
    });
    if (!res.ok) throw await errorFromResponse(res);
  }
}
