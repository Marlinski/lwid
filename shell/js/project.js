/**
 * project.js — Browser-side project manager for "lookwhatidid"
 *
 * Orchestrates the full lifecycle: creating projects, pushing new versions,
 * pulling content, and managing version history.
 *
 * Manifests store file paths as AES-256-GCM ciphertext (schema version 100).
 * File contents are encrypted with AES-256-GCM using the read key.
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
import { splitFragment, pageSuffix } from "./page-link.js";
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
 * manifest with encrypted paths (schema version 100), uploads it, then
 * signs the manifest CID and updates the project root.
 *
 * @param {import('./api.js').ApiClient} api — API client instance
 * @param {string} projectId — project identifier
 * @param {string} readKey — base64url-encoded AES-256 key
 * @param {string} writeKey — base64url-encoded PKCS8 Ed25519 private key
 * A file given as `{ path, cid, size }` (no `content`) is referenced as-is:
 * its blob is already encrypted under this same read key, so carrying an
 * unchanged file over to the next version never needs it downloaded.
 *
 * @param {Array<{ path: string, content: Uint8Array } | { path: string, cid: string, size: number }>} files — files to include in this version
 * @returns {Promise<{ manifestCid: string, version: number, files: Array<{ path: string, cid: string, size: number }> }>}
 */
export async function pushVersion(api, projectId, readKey, writeKey, files) {
  // 1. Encrypt each file, compute CID, upload if needed
  /** @type {Array<{ path: string, cid: string, size: number }>} */
  const fileEntries = [];

  for (const file of files) {
    if (!file.content && file.cid) {
      fileEntries.push({ path: file.path, cid: file.cid, size: file.size });
      continue;
    }
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

  // 2. Determine parent CID
  let parentCid = null;
  const project = await api.getProject(projectId);
  if (project.root_cid) {
    parentCid = project.root_cid;
  }

  // 3. Build, serialize, and upload the new manifest (paths encrypted)
  const manifest = await createManifest(fileEntries, parentCid, readKey);
  const manifestBytes = serializeManifest(manifest);
  const manifestCid = await computeCid(manifestBytes);
  await api.uploadBlob(manifestBytes);

  // 4. Sign the manifest CID and update the project root
  const cidBytes = new TextEncoder().encode(manifestCid);
  const signature = await sign(writeKey, cidBytes);
  const signatureBase64 = toStandardBase64(signature);
  await api.updateRoot(projectId, manifestCid, signatureBase64);

  return { manifestCid, version: manifest.version, files: fileEntries };
}

// ---------------------------------------------------------------------------
// Pull
// ---------------------------------------------------------------------------

/**
 * The files of one manifest, decrypted lazily: listing them needs nothing
 * but the manifest itself, and each file's blob is only fetched and
 * decrypted the first time {@link ProjectFiles#get} asks for it — so opening
 * a project full of large media doesn't wait on every byte of it first.
 *
 * Decrypted contents are cached by CID; pass a previous instance's `cache`
 * to carry what's already been decrypted over to a newer version.
 */
export class ProjectFiles {
  /**
   * @param {import('./api.js').ApiClient} api
   * @param {string} readKey — base64url-encoded AES-256 key
   * @param {Array<{ path: string, cid: string, size: number }>} entries
   * @param {Map<string, Promise<Uint8Array>>} [cache] — CID -> decrypted content
   */
  constructor(api, readKey, entries, cache = new Map()) {
    this.api = api;
    this.readKey = readKey;
    this.cache = cache;
    /** @type {Map<string, { path: string, cid: string, size: number }>} */
    this.byPath = new Map(entries.map((e) => [e.path, { path: e.path, cid: e.cid, size: e.size }]));
  }

  get size() { return this.byPath.size; }
  has(path) { return this.byPath.has(path); }
  keys() { return this.byPath.keys(); }
  /** Manifest entries `{ path, cid, size }`, in manifest order. */
  entries() { return Array.from(this.byPath.values()); }
  entry(path) { return this.byPath.get(path); }
  /** Plaintext size in bytes, straight from the manifest. */
  sizeOf(path) { return this.byPath.get(path)?.size ?? 0; }

  /**
   * Decrypted content of `path`, fetched on first use. Concurrent callers
   * share one fetch; a failed one is forgotten so the next call retries.
   *
   * @param {string} path
   * @returns {Promise<Uint8Array>}
   */
  get(path) {
    const entry = this.byPath.get(path);
    if (!entry) return Promise.reject(new Error(`no such file: ${path}`));
    let pending = this.cache.get(entry.cid);
    if (!pending) {
      pending = this.api.getBlob(entry.cid)
        .then((blob) => decrypt(this.readKey, new Uint8Array(blob)));
      pending.catch(() => {
        if (this.cache.get(entry.cid) === pending) this.cache.delete(entry.cid);
      });
      this.cache.set(entry.cid, pending);
    }
    return pending;
  }

  /** Record content already known for `cid` (e.g. just pushed). */
  seed(cid, content) {
    this.cache.set(cid, Promise.resolve(content));
  }
}

/**
 * Pull a specific version by its manifest CID.
 *
 * Fetches the manifest (decrypting paths with the read key). File contents
 * are not fetched here — see {@link ProjectFiles}.
 *
 * @param {import('./api.js').ApiClient} api — API client instance
 * @param {string} readKey — base64url-encoded AES-256 key
 * @param {string} manifestCid — CID of the manifest to pull
 * @returns {Promise<{ manifest: object, files: ProjectFiles }>}
 */
export async function pullVersion(api, readKey, manifestCid) {
  const manifestBlob = await api.getBlob(manifestCid);
  const manifest = await deserializeManifest(new Uint8Array(manifestBlob), readKey);
  return { manifest, files: new ProjectFiles(api, readKey, manifest.files) };
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
 * @returns {Promise<{ manifest: object, files: ProjectFiles } | null>}
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
 * for each version. The read key is required to decrypt file paths stored
 * in schema version >= 100 manifests.
 *
 * @param {import('./api.js').ApiClient} api — API client instance
 * @param {string} projectId — project identifier
 * @param {string} readKey — base64url-encoded AES-256 key for decrypting paths
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
 * The fragment never leaves the browser, so the server cannot see the keys
 * — or the page, when the link points to one (see page-link.js).
 *
 * @param {string} baseUrl — origin (e.g. "https://lookwhatidid.xyz")
 * @param {string} projectId — project identifier
 * @param {string} readKey — base64url-encoded AES-256 key
 * @param {string} [writeKey] — base64url-encoded PKCS8 private key (omit for view-only)
 * @param {{ path: string, query?: string } | null} [page] — a page inside the project to open (omit for the first page)
 * @returns {string} full URL with keys (and page) in the fragment
 */
export function buildProjectUrl(baseUrl, projectId, readKey, writeKey, page = null) {
  const base = baseUrl.replace(/\/+$/, "");
  const fragment = encodeKeys(readKey, writeKey) + pageSuffix(page);
  return `${base}/p/${projectId}#${fragment}`;
}

/**
 * Parse a project URL to extract the project ID and keys.
 *
 * Expected pattern: `{origin}/p/{projectId}#{readKey}:{writeKey}[/{page}]`
 * The write key portion (`:writeKey`) is optional for view-only links, and
 * the page portion is optional for links to the project's first page.
 *
 * @param {string} url — full project URL
 * @returns {{ projectId: string, readKey: string, writeKey: string | null, page: { path: string, query: string } | null }}
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

  const { keys, page } = splitFragment(fragment);
  const { readKey, writeKey } = decodeKeys(keys);
  return { projectId, readKey, writeKey, page };
}
