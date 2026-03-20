/**
 * crypto.js — Browser-side cryptography for "lookwhatidid"
 *
 * AES-256-GCM encryption/decryption and Ed25519 key generation/signing
 * using the Web Crypto API. Pure ES module, no external dependencies.
 */

// ---------------------------------------------------------------------------
// Base64url helpers
// ---------------------------------------------------------------------------

/**
 * Encode a Uint8Array to a base64url string (no padding).
 * @param {Uint8Array} uint8Array
 * @returns {string}
 */
export function toBase64Url(uint8Array) {
  const binary = String.fromCharCode(...uint8Array);
  return btoa(binary)
    .replace(/\+/g, "-")
    .replace(/\//g, "_")
    .replace(/=+$/, "");
}

/**
 * Decode a base64url string to a Uint8Array.
 * @param {string} str
 * @returns {Uint8Array}
 */
export function fromBase64Url(str) {
  // Restore standard base64 alphabet and padding
  let base64 = str.replace(/-/g, "+").replace(/_/g, "/");
  while (base64.length % 4 !== 0) {
    base64 += "=";
  }
  const binary = atob(base64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) {
    bytes[i] = binary.charCodeAt(i);
  }
  return bytes;
}

// ---------------------------------------------------------------------------
// AES-256-GCM encryption / decryption
// ---------------------------------------------------------------------------

/** IV length in bytes for AES-GCM. */
const IV_LENGTH = 12;

/**
 * Import a base64url-encoded 256-bit key as a CryptoKey for AES-GCM.
 * @param {string} readKeyB64url
 * @returns {Promise<CryptoKey>}
 */
async function importAesKey(readKeyB64url) {
  const rawKey = fromBase64Url(readKeyB64url);
  return crypto.subtle.importKey("raw", rawKey, { name: "AES-GCM" }, false, [
    "encrypt",
    "decrypt",
  ]);
}

/**
 * Generate a random 256-bit AES key and return it as a base64url string.
 * @returns {string}
 */
export function generateReadKey() {
  const key = new Uint8Array(32);
  crypto.getRandomValues(key);
  return toBase64Url(key);
}

/**
 * Encrypt plaintext with AES-256-GCM.
 *
 * Returns a Uint8Array with the 12-byte IV prepended to the ciphertext
 * (which includes the GCM authentication tag).
 *
 * @param {string} readKeyB64url — base64url-encoded 256-bit key
 * @param {Uint8Array} plaintext
 * @returns {Promise<Uint8Array>} iv || ciphertext
 */
export async function encrypt(readKeyB64url, plaintext) {
  const key = await importAesKey(readKeyB64url);
  const iv = crypto.getRandomValues(new Uint8Array(IV_LENGTH));

  const ciphertext = await crypto.subtle.encrypt(
    { name: "AES-GCM", iv },
    key,
    plaintext,
  );

  // Prepend IV to ciphertext
  const result = new Uint8Array(IV_LENGTH + ciphertext.byteLength);
  result.set(iv, 0);
  result.set(new Uint8Array(ciphertext), IV_LENGTH);
  return result;
}

/**
 * Decrypt an AES-256-GCM encrypted payload.
 *
 * Expects the first 12 bytes to be the IV, followed by the ciphertext
 * (including the GCM authentication tag).
 *
 * @param {string} readKeyB64url — base64url-encoded 256-bit key
 * @param {Uint8Array} encrypted — iv || ciphertext
 * @returns {Promise<Uint8Array>} plaintext
 */
export async function decrypt(readKeyB64url, encrypted) {
  const key = await importAesKey(readKeyB64url);
  const iv = encrypted.slice(0, IV_LENGTH);
  const ciphertext = encrypted.slice(IV_LENGTH);

  const plaintext = await crypto.subtle.decrypt(
    { name: "AES-GCM", iv },
    key,
    ciphertext,
  );

  return new Uint8Array(plaintext);
}

// ---------------------------------------------------------------------------
// Ed25519 key generation & signing
// ---------------------------------------------------------------------------

/**
 * Generate an Ed25519 keypair.
 *
 * Requires browser support for the "Ed25519" algorithm name
 * (Chrome 113+, Firefox 130+).
 *
 * @returns {Promise<{ publicKeyBytes: Uint8Array, privateKeyB64url: string }>}
 */
export async function generateWriteKeyPair() {
  const keyPair = await crypto.subtle.generateKey("Ed25519", true, [
    "sign",
    "verify",
  ]);

  const publicKeyRaw = await crypto.subtle.exportKey("raw", keyPair.publicKey);
  const privateKeyPkcs8 = await crypto.subtle.exportKey(
    "pkcs8",
    keyPair.privateKey,
  );

  return {
    publicKeyBytes: new Uint8Array(publicKeyRaw),
    privateKeyB64url: toBase64Url(new Uint8Array(privateKeyPkcs8)),
  };
}

/**
 * Import a PKCS8-encoded Ed25519 private key from a base64url string.
 * @param {string} privateKeyB64url
 * @returns {Promise<CryptoKey>}
 */
async function importEd25519PrivateKey(privateKeyB64url) {
  const pkcs8 = fromBase64Url(privateKeyB64url);
  return crypto.subtle.importKey("pkcs8", pkcs8, "Ed25519", true, ["sign"]);
}

/**
 * Sign a message with an Ed25519 private key.
 *
 * @param {string} privateKeyB64url — base64url-encoded PKCS8 private key
 * @param {Uint8Array} message
 * @returns {Promise<Uint8Array>} 64-byte signature
 */
export async function sign(privateKeyB64url, message) {
  const privateKey = await importEd25519PrivateKey(privateKeyB64url);
  const signature = await crypto.subtle.sign("Ed25519", privateKey, message);
  return new Uint8Array(signature);
}

/**
 * Derive the 32-byte public key from an Ed25519 private key.
 *
 * Re-imports the private key, then exports it as a JWK which contains
 * the public component ("x" parameter).
 *
 * @param {string} privateKeyB64url — base64url-encoded PKCS8 private key
 * @returns {Promise<Uint8Array>} 32-byte public key
 */
export async function getPublicKeyBytes(privateKeyB64url) {
  const privateKey = await importEd25519PrivateKey(privateKeyB64url);

  // JWK export includes the public key as the "x" parameter (base64url)
  const jwk = await crypto.subtle.exportKey("jwk", privateKey);
  return fromBase64Url(jwk.x);
}

// ---------------------------------------------------------------------------
// Store authentication
// ---------------------------------------------------------------------------

/**
 * Derive an authentication token for the store from the read key.
 *
 * Computes `SHA-256("lwid-store-auth:" + readKeyB64url)` and returns
 * the hash as a **standard base64** string (with `+`, `/`, and `=`
 * padding) to match server expectations.
 *
 * @param {string} readKeyB64url — base64url-encoded 256-bit AES key
 * @returns {Promise<string>} standard base64-encoded SHA-256 hash
 */
export async function deriveStoreToken(readKeyB64url) {
  const data = new TextEncoder().encode('lwid-store-auth:' + readKeyB64url);
  const hash = await crypto.subtle.digest('SHA-256', data);
  // Return as standard base64 (not base64url) to match server expectations
  return btoa(String.fromCharCode(...new Uint8Array(hash)));
}

// ---------------------------------------------------------------------------
// Key serialization for URL fragment
// ---------------------------------------------------------------------------

/**
 * Encode read and write keys into a URL-fragment-safe string.
 *
 * Format: `readKey:writeKey`
 *
 * If only the read key is provided (view-only), the fragment is just the
 * read key with no colon.
 *
 * @param {string} readKeyB64url
 * @param {string} [writePrivKeyB64url]
 * @returns {string}
 */
export function encodeKeys(readKeyB64url, writePrivKeyB64url) {
  if (writePrivKeyB64url) {
    return `${readKeyB64url}:${writePrivKeyB64url}`;
  }
  return readKeyB64url;
}

/**
 * Parse a URL fragment into read and (optional) write keys.
 *
 * @param {string} fragment — the raw hash fragment (without leading '#')
 * @returns {{ readKey: string, writeKey: string | null }}
 */
export function decodeKeys(fragment) {
  const colonIndex = fragment.indexOf(":");
  if (colonIndex === -1) {
    return { readKey: fragment, writeKey: null };
  }
  return {
    readKey: fragment.slice(0, colonIndex),
    writeKey: fragment.slice(colonIndex + 1),
  };
}
