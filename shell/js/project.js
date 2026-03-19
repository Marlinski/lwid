/**
 * project.js — Browser-side project manager for "lookwhatidid"
 *
 * Orchestrates the full lifecycle: creating projects, pushing new versions,
 * pulling content, and managing version history.
 *
 * Pure ES module, no external dependencies beyond local modules.
 */

import {
  generateReadKey,
  encrypt,
  decrypt,
  generateWriteKeyPair,
  sign,
  toBase64Url,
  encodeKeys,
  decodeKeys,
} from "./crypto.js";
import { computeCid } from "./cid.js";
import {
  createManifest,
  serializeManifest,
  deserializeManifest,
  walkVersionChain,
} from "./manifest.js";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/**
 * Convert a Uint8Array to standard base64 (RFC 4648, with +/= characters).
 * Safe for small buffers (keys, signatures up to ~64 bytes).
 *
 * @param {Uint8Array} bytes
 * @returns {string}
 */
function toStandardBase64(bytes) {
  return btoa(String.fromCharCode(...bytes));
}

// ---------------------------------------------------------------------------
// Project creation
// ---------------------------------------------------------------------------

/**
 * Create a new project on the server.
 *
 * Generates a read key (AES-256-GCM) for encrypting content and a write
 * keypair (Ed25519) for signing root updates. Registers the public key
 * with the server.
 *
 * @param {import('./api.js').ApiClient} api — API client instance
 * @returns {Promise<{ projectId: string, readKey: string, writeKey: string, publicKeyBytes: Uint8Array }>}
 *   - `projectId` — server-assigned project identifier
 *   - `readKey` — base64url-encoded AES-256 key
 *   - `writeKey` — base64url-encoded PKCS8 Ed25519 private key
 *   - `publicKeyBytes` — raw 32-byte Ed25519 public key
 */
export async function createNewProject(api) {
  const readKey = generateReadKey();
  const { publicKeyBytes, privateKeyB64url } = await generateWriteKeyPair();

  // Server expects standard base64, not base64url
  const pubkeyBase64 = toStandardBase64(publicKeyBytes);
  const { project_id } = await api.createProject(pubkeyBase64);

  return {
    projectId: project_id,
    readKey,
    writeKey: privateKeyB64url,
    publicKeyBytes,
  };
}

// ---------------------------------------------------------------------------
// Push
// ---------------------------------------------------------------------------

/**
 * Push a new version of the project's file tree.
 *
 * Encrypts each file, uploads blobs that don't already exist, builds a
 * manifest, encrypts and uploads it, then signs the manifest CID and
 * updates the project root.
 *
 * @param {import('./api.js').ApiClient} api — API client instance
 * @param {string} projectId — project identifier
 * @param {string} readKey — base64url-encoded AES-256 key
 * @param {string} writeKey — base64url-encoded PKCS8 Ed25519 private key
 * @param {Array<{ path: string, content: Uint8Array }>} files — files to include in this version
 * @returns {Promise<{ manifestCid: string, version: number }>}
 */
export async function pushVersion(api, projectId, readKey, writeKey, files) {
  // 1. Encrypt each file, compute CID, upload if needed
  /** @type {Array<{ path: string, cid: string, size: number }>} */
  const fileEntries = [];

  for (const file of files) {
    const encryptedBytes = await encrypt(readKey, file.content);
    const cid = await computeCid(encryptedBytes);
    const exists = await api.blobExists(cid);
    if (!exists) {
      await api.uploadBlob(encryptedBytes);
    }
    fileEntries.push({
      path: file.path,
      cid,
      size: file.content.byteLength,
    });
  }

  // 2. Determine version number and parent CID
  let version = 1;
  let parentCid = null;

  const project = await api.getProject(projectId);
  if (project.root_cid) {
    parentCid = project.root_cid;
    // Fetch and decrypt current manifest to read its version
    const encryptedManifest = await api.getBlob(project.root_cid);
    const manifestBytes = await decrypt(readKey, encryptedManifest);
    const currentManifest = deserializeManifest(manifestBytes);
    version = currentManifest.version + 1;
  }

  // 3. Build, serialize, encrypt, and upload the new manifest
  const manifest = createManifest(fileEntries, parentCid, version);
  const manifestPlaintext = serializeManifest(manifest);
  const encryptedManifest = await encrypt(readKey, manifestPlaintext);
  const manifestCid = await computeCid(encryptedManifest);
  await api.uploadBlob(encryptedManifest);

  // 4. Sign the manifest CID and update the project root
  const cidBytes = new TextEncoder().encode(manifestCid);
  const signature = await sign(writeKey, cidBytes);
  const signatureBase64 = toStandardBase64(signature);
  await api.updateRoot(projectId, manifestCid, signatureBase64);

  return { manifestCid, version };
}

// ---------------------------------------------------------------------------
// Pull
// ---------------------------------------------------------------------------

/**
 * Pull a specific version by its manifest CID.
 *
 * Fetches and decrypts the manifest, then fetches and decrypts every
 * file referenced by it.
 *
 * @param {import('./api.js').ApiClient} api — API client instance
 * @param {string} readKey — base64url-encoded AES-256 key
 * @param {string} manifestCid — CID of the manifest to pull
 * @returns {Promise<{ manifest: object, files: Map<string, Uint8Array> }>}
 *   - `manifest` — the deserialized manifest object
 *   - `files` — map from file path to decrypted content
 */
export async function pullVersion(api, readKey, manifestCid) {
  // Fetch and decrypt the manifest
  const encryptedManifest = await api.getBlob(manifestCid);
  const manifestBytes = await decrypt(readKey, encryptedManifest);
  const manifest = deserializeManifest(manifestBytes);

  // Fetch and decrypt each file
  /** @type {Map<string, Uint8Array>} */
  const files = new Map();
  for (const entry of manifest.files) {
    const encryptedBlob = await api.getBlob(entry.cid);
    const plaintext = await decrypt(readKey, encryptedBlob);
    files.set(entry.path, plaintext);
  }

  return { manifest, files };
}

/**
 * Pull the latest version of a project.
 *
 * Fetches the project to find the current root CID, then delegates to
 * {@link pullVersion}.
 *
 * @param {import('./api.js').ApiClient} api — API client instance
 * @param {string} projectId — project identifier
 * @param {string} readKey — base64url-encoded AES-256 key
 * @returns {Promise<{ manifest: object, files: Map<string, Uint8Array> } | null>}
 *   `null` if the project has no published versions yet.
 */
export async function pullLatest(api, projectId, readKey) {
  const project = await api.getProject(projectId);
  if (!project.root_cid) {
    return null;
  }
  return pullVersion(api, readKey, project.root_cid);
}

// ---------------------------------------------------------------------------
// Version history
// ---------------------------------------------------------------------------

/**
 * Retrieve the full version history of a project.
 *
 * Walks the manifest chain from newest to oldest, returning metadata
 * for each version.
 *
 * @param {import('./api.js').ApiClient} api — API client instance
 * @param {string} projectId — project identifier
 * @param {string} readKey — base64url-encoded AES-256 key
 * @returns {Promise<Array<{ version: number, timestamp: string, cid: string }>>}
 *   Array ordered from newest to oldest.
 */
export async function getVersionHistory(api, projectId, readKey) {
  const project = await api.getProject(projectId);
  if (!project.root_cid) {
    return [];
  }
  return walkVersionChain(project.root_cid, api, readKey);
}

// ---------------------------------------------------------------------------
// URL management
// ---------------------------------------------------------------------------

/**
 * Build a full project URL with keys encoded in the fragment.
 *
 * The fragment never leaves the browser, so the server cannot see the keys.
 *
 * @param {string} baseUrl — origin (e.g. "https://lookwhatidid.ovh")
 * @param {string} projectId — project identifier
 * @param {string} readKey — base64url-encoded AES-256 key
 * @param {string} [writeKey] — base64url-encoded PKCS8 private key (omit for view-only)
 * @returns {string} full URL with keys in the fragment
 */
export function buildProjectUrl(baseUrl, projectId, readKey, writeKey) {
  const base = baseUrl.replace(/\/+$/, "");
  const fragment = encodeKeys(readKey, writeKey);
  return `${base}/p/${projectId}#${fragment}`;
}

/**
 * Parse a project URL to extract the project ID and keys.
 *
 * Expected pattern: `{origin}/p/{projectId}#{readKey}:{writeKey}`
 * The write key portion (`:writeKey`) is optional for view-only links.
 *
 * @param {string} url — full project URL
 * @returns {{ projectId: string, readKey: string, writeKey: string | null }}
 * @throws {Error} if the URL does not match the expected pattern
 */
export function parseProjectUrl(url) {
  const parsed = new URL(url);
  const pathMatch = parsed.pathname.match(/\/p\/([^/]+)/);
  if (!pathMatch) {
    throw new Error(`Invalid project URL: path does not match /p/{projectId}`);
  }
  const projectId = decodeURIComponent(pathMatch[1]);

  const fragment = parsed.hash.replace(/^#/, "");
  if (!fragment) {
    throw new Error("Invalid project URL: missing fragment with keys");
  }

  const { readKey, writeKey } = decodeKeys(fragment);
  return { projectId, readKey, writeKey };
}
