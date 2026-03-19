// cid.js — Browser-side CIDv1 computation (content-addressed identifiers).
//
// Produces identical CIDv1 strings as the Rust server (`src/cid.rs`).
//
// Pipeline:
//   1. SHA-256 hash of the raw bytes.
//   2. Wrap in a multihash: varint(0x12) || varint(0x20) || sha256_digest.
//   3. Prepend CID version varint(0x01) and codec varint(0x55 = raw).
//   4. Encode the binary CID with multibase base32lower (prefix 'b').
//
// No external dependencies — uses SubtleCrypto for SHA-256 and manual
// base32lower / unsigned-varint implementations.

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/** CID version 1. */
const CID_VERSION = 0x01;
/** Multicodec: raw binary. */
const RAW_CODEC = 0x55;
/** Multihash: SHA2-256 function code. */
const SHA2_256_CODE = 0x12;
/** SHA-256 digest length in bytes. */
const SHA2_256_LEN = 0x20;

/** RFC 4648 base32 lowercase alphabet. */
const BASE32_ALPHABET = "abcdefghijklmnopqrstuvwxyz234567";

// ---------------------------------------------------------------------------
// Unsigned varint
// ---------------------------------------------------------------------------

/**
 * Encode a non-negative integer as an unsigned varint (LEB128 unsigned).
 *
 * Values < 128 are a single byte. Larger values use MSB continuation-bit
 * encoding: each byte carries 7 payload bits, with bit 7 set on all bytes
 * except the last.
 *
 * @param {number} value — non-negative integer to encode (safe integer range)
 * @returns {Uint8Array} the varint bytes
 */
export function encodeVarint(value) {
  if (value < 0) throw new RangeError("varint value must be non-negative");

  const bytes = [];
  while (value >= 0x80) {
    bytes.push((value & 0x7f) | 0x80);
    value >>>= 7;
  }
  bytes.push(value & 0x7f);
  return new Uint8Array(bytes);
}

/**
 * Decode one unsigned varint from the front of a Uint8Array.
 *
 * @param {Uint8Array} buf
 * @param {number} offset — byte offset to start reading from
 * @returns {{ value: number, bytesRead: number }}
 * @throws {Error} if the varint is truncated or overflows safe integer range
 */
function decodeVarint(buf, offset = 0) {
  let value = 0;
  let shift = 0;
  let bytesRead = 0;

  while (offset < buf.length) {
    const byte = buf[offset];
    offset++;
    bytesRead++;

    value |= (byte & 0x7f) << shift;
    if ((byte & 0x80) === 0) {
      return { value: value >>> 0, bytesRead };
    }
    shift += 7;
    if (shift > 49) {
      throw new Error("varint overflow: exceeds safe integer range");
    }
  }
  throw new Error("varint decode error: unexpected end of data");
}

// ---------------------------------------------------------------------------
// Base32lower (RFC 4648, lowercase, no padding, multibase prefix 'b')
// ---------------------------------------------------------------------------

/**
 * Encode bytes to base32lower string (no padding).
 *
 * @param {Uint8Array} data
 * @returns {string} base32lower encoded string (without multibase prefix)
 */
export function base32lowerEncode(data) {
  let bits = 0;
  let buffer = 0;
  let out = "";

  for (let i = 0; i < data.length; i++) {
    buffer = (buffer << 8) | data[i];
    bits += 8;
    while (bits >= 5) {
      bits -= 5;
      out += BASE32_ALPHABET[(buffer >>> bits) & 0x1f];
    }
  }
  // Flush remaining bits (if any), left-padded with zeros.
  if (bits > 0) {
    out += BASE32_ALPHABET[(buffer << (5 - bits)) & 0x1f];
  }
  return out;
}

/**
 * Decode a base32lower string (no padding) to bytes.
 *
 * @param {string} str — base32lower encoded string (without multibase prefix)
 * @returns {Uint8Array}
 * @throws {Error} on invalid characters
 */
export function base32lowerDecode(str) {
  // Build reverse lookup on first call.
  const lookup = new Uint8Array(128);
  lookup.fill(0xff);
  for (let i = 0; i < BASE32_ALPHABET.length; i++) {
    lookup[BASE32_ALPHABET.charCodeAt(i)] = i;
  }

  let bits = 0;
  let buffer = 0;
  const out = [];

  for (let i = 0; i < str.length; i++) {
    const code = str.charCodeAt(i);
    const val = code < 128 ? lookup[code] : 0xff;
    if (val === 0xff) {
      throw new Error(`base32lower decode: invalid character '${str[i]}' at index ${i}`);
    }
    buffer = (buffer << 5) | val;
    bits += 5;
    if (bits >= 8) {
      bits -= 8;
      out.push((buffer >>> bits) & 0xff);
    }
  }
  return new Uint8Array(out);
}

// ---------------------------------------------------------------------------
// CID computation
// ---------------------------------------------------------------------------

/**
 * Compute the CIDv1 (raw codec, SHA-256) for the given bytes.
 *
 * The result is a multibase base32lower string (prefix `b`) that is
 * byte-for-byte identical to the Rust server's `Cid::from_bytes()`.
 *
 * @param {Uint8Array} data — raw bytes to address
 * @returns {Promise<string>} the CIDv1 multibase string
 */
export async function computeCid(data) {
  // 1. SHA-256 digest.
  const digestBuf = await crypto.subtle.digest("SHA-256", data);
  const digest = new Uint8Array(digestBuf);

  // 2. Build CIDv1 binary:
  //    varint(1) || varint(0x55) || varint(0x12) || varint(0x20) || digest
  const version = encodeVarint(CID_VERSION);
  const codec = encodeVarint(RAW_CODEC);
  const hashCode = encodeVarint(SHA2_256_CODE);
  const hashLen = encodeVarint(SHA2_256_LEN);

  const binary = new Uint8Array(
    version.length + codec.length + hashCode.length + hashLen.length + digest.length,
  );
  let offset = 0;
  binary.set(version, offset);
  offset += version.length;
  binary.set(codec, offset);
  offset += codec.length;
  binary.set(hashCode, offset);
  offset += hashCode.length;
  binary.set(hashLen, offset);
  offset += hashLen.length;
  binary.set(digest, offset);

  // 3. Multibase base32lower: prefix 'b' + base32lower(binary).
  return "b" + base32lowerEncode(binary);
}

// ---------------------------------------------------------------------------
// CID validation
// ---------------------------------------------------------------------------

/**
 * Validate a CID string.
 *
 * Decodes the multibase base32lower encoding, parses the varint fields, and
 * checks the structure matches CIDv1 / raw codec / SHA-256.
 *
 * @param {string} cidStr — the multibase CID string to validate
 * @returns {boolean} true if valid, false otherwise
 */
export function validateCid(cidStr) {
  try {
    // Must start with 'b' (base32lower multibase prefix).
    if (!cidStr || cidStr[0] !== "b") return false;

    const bytes = base32lowerDecode(cidStr.slice(1));
    if (bytes.length < 4) return false;

    let pos = 0;

    // Version.
    const ver = decodeVarint(bytes, pos);
    pos += ver.bytesRead;
    if (ver.value !== CID_VERSION) return false;

    // Codec.
    const cod = decodeVarint(bytes, pos);
    pos += cod.bytesRead;
    if (cod.value !== RAW_CODEC) return false;

    // Hash function code.
    const hf = decodeVarint(bytes, pos);
    pos += hf.bytesRead;
    if (hf.value !== SHA2_256_CODE) return false;

    // Digest length.
    const dl = decodeVarint(bytes, pos);
    pos += dl.bytesRead;
    if (dl.value !== SHA2_256_LEN) return false;

    // Remaining bytes must be exactly the digest.
    if (bytes.length - pos !== SHA2_256_LEN) return false;

    return true;
  } catch {
    return false;
  }
}
