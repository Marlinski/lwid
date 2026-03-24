//! Client-side cryptography: AES-256-GCM encryption/decryption.
//!
//! Mirrors the browser's Web Crypto API operations so the CLI can
//! encrypt/decrypt identically to the shell SPA.

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Key, Nonce,
};
use base64::prelude::*;
use rand::rngs::OsRng;
use rand::RngCore;
use thiserror::Error;

/// 96-bit (12-byte) nonce for AES-GCM.
const NONCE_LEN: usize = 12;

#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("encryption failed: {0}")]
    Encrypt(String),

    #[error("decryption failed: {0}")]
    Decrypt(String),

    #[error("invalid key length: expected 32, got {0}")]
    InvalidKeyLength(usize),

    #[error("ciphertext too short")]
    CiphertextTooShort,
}

/// Generate a random 256-bit AES key.
pub fn generate_read_key() -> [u8; 32] {
    let mut key = [0u8; 32];
    OsRng.fill_bytes(&mut key);
    key
}

/// Encrypt `plaintext` with AES-256-GCM.
///
/// Returns `nonce || ciphertext` (12 bytes nonce prepended).
/// This matches the browser's Web Crypto format.
pub fn encrypt(key: &[u8; 32], plaintext: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));

    let mut nonce_bytes = [0u8; NONCE_LEN];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| CryptoError::Encrypt(e.to_string()))?;

    // Prepend nonce to ciphertext
    let mut result = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    result.extend_from_slice(&nonce_bytes);
    result.extend_from_slice(&ciphertext);
    Ok(result)
}

/// Decrypt `data` (nonce || ciphertext) with AES-256-GCM.
pub fn decrypt(key: &[u8; 32], data: &[u8]) -> Result<Vec<u8>, CryptoError> {
    if data.len() < NONCE_LEN {
        return Err(CryptoError::CiphertextTooShort);
    }

    let (nonce_bytes, ciphertext) = data.split_at(NONCE_LEN);
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let nonce = Nonce::from_slice(nonce_bytes);

    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| CryptoError::Decrypt(e.to_string()))
}

/// Encrypt a file path string using AES-256-GCM and return it as a base64url string.
///
/// Format: base64url(nonce || ciphertext) — same wire format as file blobs.
pub fn encrypt_path(key: &[u8; 32], path: &str) -> Result<String, CryptoError> {
    let encrypted = encrypt(key, path.as_bytes())?;
    Ok(BASE64_URL_SAFE_NO_PAD.encode(&encrypted))
}

/// Decrypt a base64url-encoded encrypted path back to a UTF-8 string.
pub fn decrypt_path(key: &[u8; 32], encoded: &str) -> Result<String, CryptoError> {
    let bytes = BASE64_URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|e| CryptoError::Decrypt(format!("base64 decode: {e}")))?;
    let plaintext = decrypt(key, &bytes)?;
    String::from_utf8(plaintext).map_err(|e| CryptoError::Decrypt(format!("utf8: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let key = generate_read_key();
        let plaintext = b"hello, encrypted world!";
        let encrypted = encrypt(&key, plaintext).expect("encrypt");
        let decrypted = decrypt(&key, &encrypted).expect("decrypt");
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn wrong_key_fails() {
        let key1 = generate_read_key();
        let key2 = generate_read_key();
        let encrypted = encrypt(&key1, b"secret").expect("encrypt");
        assert!(decrypt(&key2, &encrypted).is_err());
    }

    #[test]
    fn ciphertext_too_short() {
        let key = generate_read_key();
        assert!(matches!(
            decrypt(&key, &[0u8; 5]),
            Err(CryptoError::CiphertextTooShort)
        ));
    }

    #[test]
    fn different_nonces_produce_different_ciphertext() {
        let key = generate_read_key();
        let plaintext = b"same data";
        let a = encrypt(&key, plaintext).expect("encrypt a");
        let b = encrypt(&key, plaintext).expect("encrypt b");
        // Nonces are random, so ciphertext should differ
        assert_ne!(a, b);
        // But both decrypt to the same plaintext
        assert_eq!(decrypt(&key, &a).unwrap(), plaintext);
        assert_eq!(decrypt(&key, &b).unwrap(), plaintext);
    }

    #[test]
    fn encrypt_decrypt_path_roundtrip() {
        let key = generate_read_key();
        let path = "src/components/App.tsx";
        let encoded = encrypt_path(&key, path).expect("encrypt_path");
        let decoded = decrypt_path(&key, &encoded).expect("decrypt_path");
        assert_eq!(decoded, path);
    }
}
