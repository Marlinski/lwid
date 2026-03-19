/**
 * manifest.js — Browser-side manifest management for "lookwhatidid"
 *
 * A manifest describes one version (snapshot) of a project's file tree.
 * Manifests are encrypted at rest and linked into a chain via parent_cid.
 *
 * Pure ES module, no external dependencies.
 */

import { decrypt } from './crypto.js';

// ---------------------------------------------------------------------------
// Creation
// ---------------------------------------------------------------------------

/**
 * Create a new manifest object.
 *
 * @param {Array<{ path: string, cid: string, size: number }>} files
 * @param {string | null} parentCid — CID of the previous manifest, or null for the first version
 * @param {number} version — 1-based version number
 * @returns {{ version: number, parent_cid: string | null, timestamp: string, files: Array<{ path: string, cid: string, size: number }> }}
 */
export function createManifest(files, parentCid, version) {
  return {
    version,
    parent_cid: parentCid ?? null,
    timestamp: new Date().toISOString(),
    files,
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
 *
 * @param {Uint8Array} uint8Array
 * @returns {{ version: number, parent_cid: string | null, timestamp: string, files: Array<{ path: string, cid: string, size: number }> }}
 * @throws {Error} If the data is not valid JSON or required fields are missing
 */
export function deserializeManifest(uint8Array) {
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

  return manifest;
}

// ---------------------------------------------------------------------------
// Version chain traversal
// ---------------------------------------------------------------------------

/**
 * Walk the manifest version chain from newest to oldest.
 *
 * Starting from the given manifest CID, fetches each manifest blob from the
 * API, decrypts it with the read key, deserializes it, and follows the
 * parent_cid link until reaching a manifest with no parent.
 *
 * @param {string} manifestCid — CID of the root (latest) manifest
 * @param {import('./api.js').ApiClient} api — API client instance
 * @param {string} readKey — base64url-encoded AES-256 key
 * @returns {Promise<Array<{ version: number, timestamp: string, cid: string }>>}
 *   Array ordered from newest to oldest.
 */
export async function walkVersionChain(manifestCid, api, readKey) {
  /** @type {Array<{ version: number, timestamp: string, cid: string }>} */
  const chain = [];
  let currentCid = manifestCid;

  while (currentCid) {
    const encryptedBlob = await api.getBlob(currentCid);
    const plaintext = await decrypt(readKey, new Uint8Array(encryptedBlob));
    const manifest = deserializeManifest(plaintext);

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
