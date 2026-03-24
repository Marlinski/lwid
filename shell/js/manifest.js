/**
 * manifest.js — Browser-side manifest management for "lookwhatidid"
 *
 * A manifest describes one version (snapshot) of a project's file tree.
 * Manifests with version >= 100 store file paths as base64url-encoded
 * AES-256-GCM ciphertext (IV prepended). File contents remain encrypted.
 *
 * Pure ES module, no external dependencies beyond local crypto.js.
 */

import { encrypt, decrypt, toBase64Url, fromBase64Url } from './crypto.js';

/** Schema version at which path encryption was introduced. */
const SCHEMA_ENCRYPTED_PATHS = 100;

// ---------------------------------------------------------------------------
// Creation
// ---------------------------------------------------------------------------

/**
 * Create a new manifest object.
 *
 * Paths are encrypted with AES-256-GCM using the provided read key and stored
 * as base64url-encoded ciphertext (schema version 100).
 *
 * @param {Array<{ path: string, cid: string, size: number }>} files
 * @param {string | null} parentCid — CID of the previous manifest, or null for the first version
 * @param {string} readKey — base64url-encoded AES-256 key used to encrypt paths
 * @returns {Promise<{ version: number, parent_cid: string | null, timestamp: string, files: Array<{ path: string, cid: string, size: number }> }>}
 */
export async function createManifest(files, parentCid, readKey) {
  const encryptedFiles = await Promise.all(
    files.map(async (file) => {
      const pathBytes = new TextEncoder().encode(file.path);
      const encryptedBytes = await encrypt(readKey, pathBytes);
      const encryptedPath = toBase64Url(encryptedBytes);
      return { path: encryptedPath, cid: file.cid, size: file.size };
    })
  );
  return {
    version: SCHEMA_ENCRYPTED_PATHS,
    parent_cid: parentCid ?? null,
    timestamp: new Date().toISOString(),
    files: encryptedFiles,
  };
}

// ---------------------------------------------------------------------------
// Serialization
// ---------------------------------------------------------------------------

/**
 * Serialize a manifest to a UTF-8 encoded Uint8Array (JSON with 2-space indentation).
 *
 * @param {{ version: number, parent_cid: string | null, timestamp: string, files: Array<{ path: string, cid: string, size: number }> }} manifest
 * @returns {Uint8Array}
 */
export function serializeManifest(manifest) {
  const json = JSON.stringify(manifest, null, 2);
  return new TextEncoder().encode(json);
}

/**
 * Deserialize a UTF-8 encoded Uint8Array into a manifest object.
 * Validates that required fields (version, timestamp, files) are present.
 * For schema version >= 100, decrypts each file path using the provided read key.
 *
 * @param {Uint8Array} uint8Array
 * @param {string} [readKey] — base64url-encoded AES-256 key; required for version >= 100
 * @returns {Promise<{ version: number, parent_cid: string | null, timestamp: string, files: Array<{ path: string, cid: string, size: number }> }>}
 * @throws {Error} If the data is not valid JSON or required fields are missing
 */
export async function deserializeManifest(uint8Array, readKey) {
  const json = new TextDecoder().decode(uint8Array);
  const manifest = JSON.parse(json);

  if (typeof manifest.version !== 'number') {
    throw new Error('Invalid manifest: missing or invalid "version" field');
  }
  if (typeof manifest.timestamp !== 'string') {
    throw new Error('Invalid manifest: missing or invalid "timestamp" field');
  }
  if (!Array.isArray(manifest.files)) {
    throw new Error('Invalid manifest: missing or invalid "files" field');
  }

  // Decrypt paths for schema-v1 manifests
  if (manifest.version >= SCHEMA_ENCRYPTED_PATHS && readKey) {
    manifest.files = await Promise.all(
      manifest.files.map(async (entry) => {
        const encryptedBytes = fromBase64Url(entry.path);
        const pathBytes = await decrypt(readKey, encryptedBytes);
        const path = new TextDecoder().decode(pathBytes);
        return { ...entry, path };
      })
    );
  }

  return manifest;
}

// ---------------------------------------------------------------------------
// Version chain traversal
// ---------------------------------------------------------------------------

/**
 * Walk the manifest version chain from newest to oldest.
 *
 * Starting from the given manifest CID, fetches each manifest blob from the
 * API, deserializes it (decrypting paths with the provided read key), and
 * follows the parent_cid link until reaching a manifest with no parent.
 *
 * @param {string} manifestCid — CID of the root (latest) manifest
 * @param {import('./api.js').ApiClient} api — API client instance
 * @param {string} readKey — base64url-encoded AES-256 key for decrypting paths
 * @returns {Promise<Array<{ version: number, timestamp: string, cid: string }>>}
 *   Array ordered from newest to oldest.
 */
export async function walkVersionChain(manifestCid, api, readKey) {
  /** @type {Array<{ version: number, timestamp: string, cid: string }>} */
  const chain = [];
  let currentCid = manifestCid;

  while (currentCid) {
    const blob = await api.getBlob(currentCid);
    const manifest = await deserializeManifest(new Uint8Array(blob), readKey);

    chain.push({
      version: manifest.version,
      timestamp: manifest.timestamp,
      cid: currentCid,
    });

    currentCid = manifest.parent_cid || null;
  }

  return chain;
}

// ---------------------------------------------------------------------------
// Lookup
// ---------------------------------------------------------------------------

/**
 * Find a file entry in a manifest by its path.
 *
 * @param {{ files: Array<{ path: string, cid: string, size: number }> }} manifest
 * @param {string} path — file path to search for
 * @returns {{ path: string, cid: string, size: number } | null}
 */
export function findFile(manifest, path) {
  return manifest.files.find((f) => f.path === path) ?? null;
}
