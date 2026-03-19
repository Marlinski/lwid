//! CIDv1 (Content Identifier version 1) computation.
//!
//! Wraps the IPFS `cid` crate to produce content-addressed identifiers
//! using SHA-256 multihash, raw codec, base32lower multibase encoding.

use std::fmt;
use std::path::PathBuf;

use cid::multibase::Base;
use cid::CidGeneric;
use multihash_codetable::{Code, MultihashDigest};
use thiserror::Error;

/// Multicodec: raw binary.
const RAW_CODEC: u64 = 0x55;

/// The multibase encoding we use for string representation.
const STRING_BASE: Base = Base::Base32Lower;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum CidError {
    #[error("CID parse failed: {0}")]
    Parse(String),

    #[error("unsupported CID version: {0}")]
    UnsupportedVersion(u64),

    #[error("unsupported codec: 0x{0:x}")]
    UnsupportedCodec(u64),

    #[error("unsupported hash function: 0x{0:x}")]
    UnsupportedHashFunction(u64),
}

// ---------------------------------------------------------------------------
// Cid struct
// ---------------------------------------------------------------------------

/// A CIDv1 represented as its base32lower multibase string.
///
/// This is a thin wrapper providing a consistent string representation
/// and filesystem path sharding.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Cid {
    /// The inner IPFS CID.
    inner: CidGeneric<64>,
    /// Cached base32lower string representation.
    encoded: String,
}

impl Cid {
    /// Compute the CIDv1 (raw codec, SHA-256) for the given byte slice.
    pub fn from_bytes(data: &[u8]) -> Self {
        let hash = Code::Sha2_256.digest(data);
        let inner = CidGeneric::new_v1(RAW_CODEC, hash);
        let encoded = inner
            .to_string_of_base(STRING_BASE)
            .expect("base32lower encoding cannot fail");
        Self { inner, encoded }
    }

    /// Validate and construct a [`Cid`] from a multibase base32lower string.
    pub fn from_string(s: &str) -> Result<Self, CidError> {
        let inner: CidGeneric<64> = s
            .try_into()
            .map_err(|e: ::cid::Error| CidError::Parse(e.to_string()))?;

        // Validate constraints: must be CIDv1, raw codec, SHA-256
        if inner.version() != ::cid::Version::V1 {
            return Err(CidError::UnsupportedVersion(0)); // V0
        }

        if inner.codec() != RAW_CODEC {
            return Err(CidError::UnsupportedCodec(inner.codec()));
        }

        // SHA-256 multihash code is 0x12
        if inner.hash().code() != 0x12 {
            return Err(CidError::UnsupportedHashFunction(inner.hash().code()));
        }

        // Normalize to base32lower
        let encoded = inner
            .to_string_of_base(STRING_BASE)
            .expect("base32lower encoding cannot fail");

        Ok(Self { inner, encoded })
    }

    /// Return the multibase-encoded CID string.
    pub fn as_str(&self) -> &str {
        &self.encoded
    }

    /// Return a sharded filesystem path: `<first 2 chars>/<next 2 chars>/<full CID>`.
    pub fn to_path(&self) -> PathBuf {
        let s = self.as_str();
        let shard1 = &s[..2];
        let shard2 = &s[2..4];
        PathBuf::from(shard1).join(shard2).join(s)
    }
}

// Serde: serialize/deserialize as the base32lower string
impl serde::Serialize for Cid {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.encoded.serialize(serializer)
    }
}

impl<'de> serde::Deserialize<'de> for Cid {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Cid::from_string(&s).map_err(serde::de::Error::custom)
    }
}

impl fmt::Display for Cid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.encoded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_same_data() {
        let a = Cid::from_bytes(b"hello world");
        let b = Cid::from_bytes(b"hello world");
        assert_eq!(a, b);
    }

    #[test]
    fn different_data_different_cid() {
        let a = Cid::from_bytes(b"hello");
        let b = Cid::from_bytes(b"world");
        assert_ne!(a, b);
    }

    #[test]
    fn roundtrip_from_bytes_to_string_from_string() {
        let cid = Cid::from_bytes(b"roundtrip test payload");
        let s = cid.to_string();
        let parsed = Cid::from_string(&s).expect("should parse back");
        assert_eq!(cid, parsed);
    }

    #[test]
    fn starts_with_base32lower_prefix() {
        let cid = Cid::from_bytes(b"any data");
        assert!(
            cid.as_str().starts_with('b'),
            "base32lower CIDs must start with 'b'"
        );
    }

    #[test]
    fn to_path_sharding() {
        let cid = Cid::from_bytes(b"shard me");
        let path = cid.to_path();
        let s = cid.as_str();
        let expected = PathBuf::from(&s[..2]).join(&s[2..4]).join(s);
        assert_eq!(path, expected);
    }

    #[test]
    fn from_string_rejects_garbage() {
        assert!(Cid::from_string("not-a-cid").is_err());
    }

    #[test]
    fn display_matches_as_str() {
        let cid = Cid::from_bytes(b"display test");
        assert_eq!(format!("{cid}"), cid.as_str());
    }
}
