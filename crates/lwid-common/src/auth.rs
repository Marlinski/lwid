use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("invalid public key: {0}")]
    InvalidPublicKey(String),

    #[error("invalid signature: {0}")]
    InvalidSignature(String),

    #[error("signature verification failed")]
    VerificationFailed,
}

/// Verify an Ed25519 signature against a message.
///
/// - `public_key_bytes`: 32-byte Ed25519 public key
/// - `message`: the signed payload
/// - `signature_bytes`: 64-byte Ed25519 signature
pub fn verify_signature(
    public_key_bytes: &[u8],
    message: &[u8],
    signature_bytes: &[u8],
) -> Result<(), AuthError> {
    let key_bytes: [u8; 32] = public_key_bytes.try_into().map_err(|_| {
        AuthError::InvalidPublicKey(format!("expected 32 bytes, got {}", public_key_bytes.len()))
    })?;

    let verifying_key = VerifyingKey::from_bytes(&key_bytes)
        .map_err(|e| AuthError::InvalidPublicKey(e.to_string()))?;

    let sig_bytes: [u8; 64] = signature_bytes.try_into().map_err(|_| {
        AuthError::InvalidSignature(format!("expected 64 bytes, got {}", signature_bytes.len()))
    })?;

    let signature = Signature::from_bytes(&sig_bytes);

    verifying_key
        .verify(message, &signature)
        .map_err(|_| AuthError::VerificationFailed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn generate_keypair() -> (SigningKey, VerifyingKey) {
        let mut rng = rand_core::OsRng;
        let signing_key = SigningKey::generate(&mut rng);
        let verifying_key = signing_key.verifying_key();
        (signing_key, verifying_key)
    }

    #[test]
    fn valid_signature_passes() {
        let (signing_key, verifying_key) = generate_keypair();
        let message = b"hello lookwhatidid";
        let signature = signing_key.sign(message);

        let result = verify_signature(verifying_key.as_bytes(), message, &signature.to_bytes());
        assert!(result.is_ok());
    }

    #[test]
    fn wrong_message_fails() {
        let (signing_key, verifying_key) = generate_keypair();
        let message = b"correct message";
        let signature = signing_key.sign(message);

        let result = verify_signature(
            verifying_key.as_bytes(),
            b"wrong message",
            &signature.to_bytes(),
        );
        assert!(matches!(result, Err(AuthError::VerificationFailed)));
    }

    #[test]
    fn invalid_public_key_bytes_fails() {
        let message = b"some message";
        let signature_bytes = [0u8; 64];

        // Too short
        let result = verify_signature(&[0u8; 16], message, &signature_bytes);
        assert!(matches!(result, Err(AuthError::InvalidPublicKey(_))));

        // Too long
        let result = verify_signature(&[0u8; 48], message, &signature_bytes);
        assert!(matches!(result, Err(AuthError::InvalidPublicKey(_))));
    }

    #[test]
    fn invalid_signature_bytes_fails() {
        let (_signing_key, verifying_key) = generate_keypair();
        let message = b"some message";

        // Too short
        let result = verify_signature(verifying_key.as_bytes(), message, &[0u8; 32]);
        assert!(matches!(result, Err(AuthError::InvalidSignature(_))));

        // Too long
        let result = verify_signature(verifying_key.as_bytes(), message, &[0u8; 128]);
        assert!(matches!(result, Err(AuthError::InvalidSignature(_))));
    }
}
